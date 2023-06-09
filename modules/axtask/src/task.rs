use alloc::{boxed::Box, string::String, sync::Arc};
use core::ops::Deref;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, Ordering};
use core::{alloc::Layout, cell::UnsafeCell, fmt, ptr::NonNull};

#[cfg(feature = "preempt")]
use core::sync::atomic::AtomicUsize;

use axhal::arch::TaskContext;
use memory_addr::{align_up_4k, VirtAddr};

use crate::{current, AxRunQueue, AxTask, AxTaskRef, WaitQueue, RUN_QUEUE};

/// A unique identifier for a thread.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TaskId(u64);

/// The possible states of a task.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum TaskState {
    Running = 1,
    Ready = 2,
    Blocked = 3,
    Exited = 4,
}

/// The inner task structure.
pub struct TaskInner {
    id: TaskId,
    name: String,
    is_idle: bool,
    is_init: bool,

    entry: Option<*mut dyn FnOnce()>,
    state: AtomicU8,

    in_wait_queue: AtomicBool,
    #[cfg(feature = "irq")]
    in_timer_list: AtomicBool,

    #[cfg(feature = "preempt")]
    need_resched: AtomicBool,
    #[cfg(feature = "preempt")]
    preempt_disable_count: AtomicUsize,

    exit_code: AtomicI32,
    wait_for_exit: WaitQueue,

    kstack: Option<TaskStack>,
    ctx: UnsafeCell<TaskContext>,

    #[cfg(feature = "user-paging")]
    trap_frame: Option<(Arc<axalloc::GlobalPage>, VirtAddr)>,
    #[cfg(feature = "user-paging")]
    ustack: Option<(Arc<axalloc::GlobalPage>, VirtAddr)>,
    #[cfg(feature = "process")]
    pid: AtomicU64,
}

impl TaskId {
    fn new() -> Self {
        static ID_COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Convert the task ID to a `u64`.
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

impl From<u8> for TaskState {
    #[inline]
    fn from(state: u8) -> Self {
        match state {
            1 => Self::Running,
            2 => Self::Ready,
            3 => Self::Blocked,
            4 => Self::Exited,
            _ => unreachable!(),
        }
    }
}

unsafe impl Send for TaskInner {}
unsafe impl Sync for TaskInner {}

impl TaskInner {
    /// Gets the ID of the task.
    pub const fn id(&self) -> TaskId {
        self.id
    }

    /// Gets the name of the task.
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Get a combined string of the task ID and name.
    pub fn id_name(&self) -> alloc::string::String {
        alloc::format!("Task({}, {:?})", self.id.as_u64(), self.name)
    }

    /// Wait for the task to exit, and return the exit code.
    ///
    /// It will return immediately if the task has already exited (but not dropped).
    pub fn join(&self) -> Option<i32> {
        self.wait_for_exit
            .wait_until(|| self.state() == TaskState::Exited);
        Some(self.exit_code.load(Ordering::Acquire))
    }

    /// current task's pid
    #[cfg(feature = "process")]
    pub fn pid(&self) -> u64 {
        self.pid.load(Ordering::Relaxed)
    }

    /// release memory when exit
    #[cfg(feature = "process")]
    pub fn on_exit<F>(&self, remove_fn: F)
    where
        F: Fn(VirtAddr),
    {
        remove_fn(self.ustack.as_ref().unwrap().1);
        remove_fn(self.trap_frame.as_ref().unwrap().1);
    }
}

#[cfg(feature = "user-paging")]
impl TaskInner {
    fn setup_ustack(&mut self) {
        use axhal::paging::MappingFlags;

        let ustack_start = get_ustack_vaddr(self.id);
        self.ustack = Some((
            axmem::alloc_user_page(
                ustack_start,
                axmem::USTACK_SIZE,
                MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
            ),
            ustack_start,
        ));
    }

    fn setup_trapframe(&mut self, start: usize) {
        use axhal::paging::MappingFlags;
        let tf_addr = get_trap_frame_vaddr(self.id);
        let trap_frame = axmem::alloc_user_page(
            tf_addr,
            TRAP_FRAME_SIZE,
            MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
        );
        let ustack_start = self.ustack.as_ref().unwrap().1;

        // TODO: make a HAL wrapper
        #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
        unsafe {
            let trap_frame = &mut *(trap_frame.as_ptr() as *mut axhal::arch::TrapFrame);
            *trap_frame =
                axhal::arch::TrapFrame::new(start, (ustack_start + axmem::USTACK_SIZE).into());
            trap_frame.kstack = self.kstack.as_ref().unwrap().top().into();
        }
        self.trap_frame = Some((trap_frame, tf_addr));
    }
}

// private methods
impl TaskInner {
    const fn new_common(id: TaskId, name: String) -> Self {
        Self {
            id,
            name,
            is_idle: false,
            is_init: false,
            entry: None,
            state: AtomicU8::new(TaskState::Ready as u8),
            in_wait_queue: AtomicBool::new(false),
            #[cfg(feature = "irq")]
            in_timer_list: AtomicBool::new(false),
            #[cfg(feature = "preempt")]
            need_resched: AtomicBool::new(false),
            #[cfg(feature = "preempt")]
            preempt_disable_count: AtomicUsize::new(0),
            exit_code: AtomicI32::new(0),
            wait_for_exit: WaitQueue::new(),
            kstack: None,
            ctx: UnsafeCell::new(TaskContext::new()),
            #[cfg(feature = "user-paging")]
            trap_frame: None,
            #[cfg(feature = "user-paging")]
            ustack: None,
            #[cfg(feature = "process")]
            pid: AtomicU64::new(0),
        }
    }

    pub(crate) fn new<F>(entry: F, name: String, stack_size: usize) -> AxTaskRef
    where
        F: FnOnce() + Send + 'static,
    {
        let mut t = Self::new_common(TaskId::new(), name);
        debug!("new task: {}", t.id_name());
        let kstack = TaskStack::alloc(align_up_4k(stack_size));
        t.entry = Some(Box::into_raw(Box::new(entry)));
        t.ctx.get_mut().init(task_entry as usize, kstack.top());
        t.kstack = Some(kstack);
        if t.name == "idle" {
            t.is_idle = true;
        }
        Arc::new(AxTask::new(t))
    }

    #[allow(dead_code)]
    #[cfg(feature = "user-paging")]
    pub(crate) fn new_user(
        entry: usize,
        kstack_size: usize,
        #[allow(unused)] args: usize,
    ) -> AxTaskRef {
        let mut t = Self::new_common(TaskId::new(), "".into());
        debug!("new user task: {} {}", t.id_name(), entry);
        let kstack = TaskStack::alloc(align_up_4k(kstack_size));
        t.ctx.get_mut().init(task_user_entry as usize, kstack.top());
        t.kstack = Some(kstack);

        t.setup_ustack();
        t.setup_trapframe(entry);

        #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
        unsafe {
            let trap_frame =
                &mut *(t.trap_frame.as_mut().unwrap().0.as_ptr() as *mut axhal::arch::TrapFrame);
            trap_frame.regs.a0 = args;
        }

        #[cfg(feature = "process")]
        {
            t.pid = current_pid().unwrap().into();
        }

        Arc::new(AxTask::new(t))
    }

    pub(crate) fn new_init(name: String) -> AxTaskRef {
        // init_task does not change PC and SP, so `entry` and `kstack` fields are not used.
        let mut t = Self::new_common(TaskId::new(), name);
        t.is_init = true;
        if t.name == "idle" {
            t.is_idle = true;
        }
        debug!("init task: {}", t.id_name());
        #[cfg(feature = "user-paging")]
        {
            t.setup_ustack();

            let kstack = TaskStack::alloc(axconfig::TASK_STACK_SIZE);
            t.kstack = Some(kstack);

            t.setup_trapframe(axmem::USER_START);
        }

        #[cfg(feature = "process")]
        {
            t.pid = 1.into();
        }

        Arc::new(AxTask::new(t))
    }

    #[cfg(all(feature = "user-paging", feature = "process"))]
    pub(crate) fn new_exec() -> AxTaskRef {
        let mut t = Self::new_common(TaskId::new(), String::new());
        t.is_init = true;
        debug!("task exec: {}", t.id_name());

        let kstack = TaskStack::alloc(axconfig::TASK_STACK_SIZE);
        t.ctx.get_mut().init(task_user_entry as usize, kstack.top());
        t.kstack = Some(kstack);

        t.setup_ustack();

        t.setup_trapframe(axmem::USER_START);

        t.pid
            .store(current().pid.load(Ordering::Relaxed), Ordering::Relaxed);

        Arc::new(AxTask::new(t))
    }

    #[cfg(all(feature = "user-paging", feature = "process"))]
    pub(crate) fn new_fork(&self, pid: u64, mem: Arc<axmem::AddrSpace>) -> AxTaskRef {
        use axalloc::GlobalPage;
        use axhal::{mem::virt_to_phys, paging::MappingFlags};
        let mut t = Self::new_common(TaskId::new(), String::new());
        t.is_init = true;
        t.pid = pid.into();
        debug!("fork task: {} -> {}", self.id_name(), t.id_name());

        let kstack = TaskStack::alloc(axconfig::TASK_STACK_SIZE);
        t.ctx.get_mut().init(task_user_entry as usize, kstack.top());
        t.kstack = Some(kstack);

        let trap_frame = Arc::new(GlobalPage::alloc().unwrap());
        let tf_addr = get_trap_frame_vaddr(t.id);
        mem.lock()
            .add_region(
                tf_addr,
                trap_frame.start_paddr(virt_to_phys),
                trap_frame.clone(),
                MappingFlags::READ | MappingFlags::WRITE | MappingFlags::USER,
                false,
            )
            .unwrap();

        // TODO: make a HAL wrapper
        #[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
        unsafe {
            // TODO: move ustack
            let old_trap_frame = mem.lock().query(get_trap_frame_vaddr(self.id)).unwrap();
            let old_trap_frame = &*(axhal::mem::phys_to_virt(old_trap_frame).as_ptr()
                as *const axhal::arch::TrapFrame);

            let new_trap_frame = &mut *(trap_frame.as_ptr() as *mut axhal::arch::TrapFrame);
            *new_trap_frame = old_trap_frame.clone();
            new_trap_frame.kstack = t.kstack.as_ref().unwrap().top().into();
            new_trap_frame.regs.a0 = 0;
        }
        t.trap_frame = Some((trap_frame, tf_addr));

        Arc::new(AxTask::new(t))
    }

    #[inline]
    pub(crate) fn state(&self) -> TaskState {
        self.state.load(Ordering::Acquire).into()
    }

    #[inline]
    pub(crate) fn set_state(&self, state: TaskState) {
        self.state.store(state as u8, Ordering::Release)
    }

    #[inline]
    pub(crate) fn is_running(&self) -> bool {
        matches!(self.state(), TaskState::Running)
    }

    #[inline]
    pub(crate) fn is_ready(&self) -> bool {
        matches!(self.state(), TaskState::Ready)
    }

    #[inline]
    pub(crate) fn is_blocked(&self) -> bool {
        matches!(self.state(), TaskState::Blocked)
    }

    #[inline]
    pub(crate) const fn is_init(&self) -> bool {
        self.is_init
    }

    #[inline]
    pub(crate) const fn is_idle(&self) -> bool {
        self.is_idle
    }

    #[inline]
    pub(crate) fn in_wait_queue(&self) -> bool {
        self.in_wait_queue.load(Ordering::Acquire)
    }

    // Used in futex
    #[inline]
    pub(crate) fn set_in_wait_queue(&self, in_wait_queue: bool) {
        self.in_wait_queue.store(in_wait_queue, Ordering::Release);
    }

    #[inline]
    #[cfg(feature = "irq")]
    pub(crate) fn in_timer_list(&self) -> bool {
        self.in_timer_list.load(Ordering::Acquire)
    }

    #[inline]
    #[cfg(feature = "irq")]
    pub(crate) fn set_in_timer_list(&self, in_timer_list: bool) {
        self.in_timer_list.store(in_timer_list, Ordering::Release);
    }

    #[inline]
    #[cfg(feature = "preempt")]
    pub(crate) fn set_preempt_pending(&self, pending: bool) {
        self.need_resched.store(pending, Ordering::Release)
    }

    #[inline]
    #[cfg(feature = "preempt")]
    pub(crate) fn can_preempt(&self, current_disable_count: usize) -> bool {
        self.preempt_disable_count.load(Ordering::Acquire) == current_disable_count
    }

    #[inline]
    #[cfg(feature = "preempt")]
    pub(crate) fn disable_preempt(&self) {
        self.preempt_disable_count.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    #[cfg(feature = "preempt")]
    pub(crate) fn enable_preempt(&self, resched: bool) {
        if self.preempt_disable_count.fetch_sub(1, Ordering::Relaxed) == 1 && resched {
            // If current task is pending to be preempted, do rescheduling.
            Self::current_check_preempt_pending();
        }
    }

    #[cfg(feature = "preempt")]
    fn current_check_preempt_pending() {
        let curr = crate::current();
        if curr.need_resched.load(Ordering::Acquire) && curr.can_preempt(0) {
            let mut rq = crate::RUN_QUEUE.lock();
            if curr.need_resched.load(Ordering::Acquire) {
                rq.resched();
            }
        }
    }

    pub(crate) fn notify_exit(&self, exit_code: i32, rq: &mut AxRunQueue) {
        self.exit_code.store(exit_code, Ordering::Release);
        self.wait_for_exit.notify_all_locked(false, rq);
    }

    #[inline]
    pub(crate) const unsafe fn ctx_mut_ptr(&self) -> *mut TaskContext {
        self.ctx.get()
    }
}

impl fmt::Debug for TaskInner {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("TaskInner")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("state", &self.state())
            .finish()
    }
}

impl Drop for TaskInner {
    fn drop(&mut self) {
        debug!("task drop: {}", self.id_name());
    }
}

struct TaskStack {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl TaskStack {
    pub fn alloc(size: usize) -> Self {
        let layout = Layout::from_size_align(size, 16).unwrap();
        Self {
            ptr: NonNull::new(unsafe { alloc::alloc::alloc(layout) }).unwrap(),
            layout,
        }
    }

    pub const fn top(&self) -> VirtAddr {
        unsafe { core::mem::transmute(self.ptr.as_ptr().add(self.layout.size())) }
    }
}

impl Drop for TaskStack {
    fn drop(&mut self) {
        unsafe { alloc::alloc::dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

use core::mem::ManuallyDrop;

/// A wrapper of [`AxTaskRef`] as the current task.
pub struct CurrentTask(ManuallyDrop<AxTaskRef>);

impl CurrentTask {
    pub(crate) fn try_get() -> Option<Self> {
        let ptr: *const super::AxTask = axhal::cpu::current_task_ptr();
        if !ptr.is_null() {
            Some(Self(unsafe { ManuallyDrop::new(AxTaskRef::from_raw(ptr)) }))
        } else {
            None
        }
    }

    pub(crate) fn get() -> Self {
        Self::try_get().expect("current task is uninitialized")
    }

    pub(crate) fn as_task_ref(&self) -> &AxTaskRef {
        &self.0
    }

    pub(crate) fn clone(&self) -> AxTaskRef {
        self.0.deref().clone()
    }

    pub(crate) fn ptr_eq(&self, other: &AxTaskRef) -> bool {
        Arc::ptr_eq(&self.0, other)
    }

    pub(crate) unsafe fn init_current(init_task: AxTaskRef) {
        let ptr = Arc::into_raw(init_task);
        axhal::cpu::set_current_task_ptr(ptr);
    }

    pub(crate) unsafe fn set_current(prev: Self, next: AxTaskRef) {
        let Self(arc) = prev;
        ManuallyDrop::into_inner(arc); // `call Arc::drop()` to decrease prev task reference count.
        let ptr = Arc::into_raw(next);
        axhal::cpu::set_current_task_ptr(ptr);
    }
}

impl Deref for CurrentTask {
    type Target = TaskInner;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

extern "C" fn task_entry() -> ! {
    // release the lock that was implicitly held across the reschedule
    unsafe { crate::RUN_QUEUE.force_unlock() };
    #[cfg(feature = "irq")]
    axhal::arch::enable_irqs();
    let task = crate::current();
    if let Some(entry) = task.entry {
        unsafe { Box::from_raw(entry)() };
    }
    crate::exit(0);
}

#[allow(dead_code)]
#[cfg(feature = "user-paging")]
extern "C" fn task_user_entry() -> ! {
    unsafe { crate::RUN_QUEUE.force_unlock() };
    axhal::arch::disable_irqs();
    axhal::arch::first_uentry();
}

cfg_if::cfg_if! {
if #[cfg(feature = "user-paging")] {
    struct CurrentTaskIf;
    #[crate_interface::impl_interface]
    impl axhal::trap::CurrentTask for CurrentTaskIf {
        fn current_trap_frame() -> *mut axhal::arch::TrapFrame {
            crate::current().trap_frame.as_ref().unwrap().0.as_ptr()
                as *mut axhal::arch::TrapFrame
        }
        fn current_satp() -> usize {
            axmem::get_satp()
        }
        fn current_trap_frame_virt_addr() -> usize {
            crate::current().trap_frame.as_ref().unwrap().1.into()
        }
    }
    const TRAP_FRAME_BASE: usize = 0xffff_ffff_ffff_f000;
    const TRAP_FRAME_SIZE: usize = axhal::mem::PAGE_SIZE_4K;
    fn get_trap_frame_vaddr(id: TaskId) -> VirtAddr {
        (TRAP_FRAME_BASE - id.0 as usize * TRAP_FRAME_SIZE).into()
    }
    fn get_ustack_vaddr(id: TaskId) -> VirtAddr {
        (axmem::USTACK_START - id.0 as usize * axmem::USTACK_SIZE).into()
    }
}
}

cfg_if::cfg_if! {
    if #[cfg(feature = "process")] {
        use axmem::AddrSpace;
        /// Gets current process id
        pub fn current_pid() -> Option<u64> {
            crate::current_may_uninit().map(|task| task.pid.load(Ordering::Relaxed))
        }
        /// Gets current task reference
        pub fn current_task() -> AxTaskRef {
            current().as_task_ref().clone()
        }
        /// Copies current task and creates a new task in the given process
        pub fn handle_fork(pid: u64, mem: Arc<AddrSpace>) -> AxTaskRef {
            let task = crate::current().new_fork(pid, mem);
            RUN_QUEUE.lock().add_task(task.clone());
            task
        }

        /// Inits a new task for the new program and run it
        pub fn handle_exec<F>(post_fn: F) -> !
        where F: FnOnce(AxTaskRef) {

            let task = TaskInner::new_exec();
            post_fn(task.clone());

            crate::run_queue::run_exec(task)
        }
    }
}
