## 进展

本周按照计划进行 `axnet` 模块的系统迁移。

主要的工作量是将向上提供的接口和向下依赖的服务均改为 IPC 形式。

### 向下接口

主要是对 `axdriver` 中提供的 `NetDevice` 进行一个 mock，将收到的请求通过 IPC 转发给仍然在内核的真 `NetDevice`（通过 `dev:/net` 接口），主要接口包括 `send`, `recv`，可以无缝衔接给 `read`, `write`，另外就是重新写一遍 buffer 管理。

在一开始看代码的时候发现有些过时的代码，因此 pull 了最新的开发版，结果确实因为 `driver` 重构改了很多。。。

### 向上接口

向上主要实现了 TCP 接口，主要是设计 TCP 的 scheme 语义。可以采用 `open` 的 `O_CREAT` 选项区分 listen 和 connect，使用 `dup` 来使得一个正在 listen 的 TCP 连接接受新的连接（`accept`）。

在实现和调试时，发现 `DevError` 和 `AxError` 的转换比较繁琐。另外 `EAGAIN` 的等待位置也得修改，避免错误的阻塞。

### 关于合并主分支

我的编译流程和原版有很大变化，导致原来的 makefile 和 CI 文件不太能支持。

## 下一步进展

实现 fs 模块的迁移。
