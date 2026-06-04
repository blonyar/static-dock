# StaticDock 静态资源码头

StaticDock 是一个轻量、可随身携带的本地静态资源服务器。把一个文件夹挂成 HTTP 服务后，Android 真机、模拟器、同一局域网里的电脑或手机都可以直接访问其中的 JSON、图片、MP4 等资源。

适合这些场景：

- Android 课程或练习项目临时获取 JSON、图片、视频。
- 教室局域网内给一整个班级分发静态资源。
- 不想安装 Tomcat、Node.js、Python，也不想折腾云存储/备案。
- 需要 MP4 边下边播和拖动进度条。

## 特性

- 单个 Windows 可执行文件：`static-dock.exe`
- 无运行时依赖：不需要 Node.js、Python、npm 包
- 支持目录挂载、端口配置、worker 线程数和请求队列
- 支持 HTTP Range：MP4 点播、拖动进度条可用
- 支持局域网、电脑热点、Android 模拟器访问
- 提供 Windows 桌面 GUI 启动器
- 提供并发压测脚本和测试素材
- Rust 实现，默认 128 workers / 2048 queue，适合课堂临时分发

## 快速开始

普通用户建议从 GitHub Releases 下载 `static-dock-windows-x64.zip`，解压后双击：

```text
start-static-dock-gui.bat
```

如果只想用命令行窗口直接启动，双击：

```text
start-static-dock.bat
```

下载或克隆仓库后，在 Windows 上双击：

```text
start-static-dock.bat
```

它会默认挂载当前目录，并打印访问地址：

```text
PC local:          http://127.0.0.1:8000/
Phone/LAN URLs:   http://你的电脑IP:8000/
Android emulator: http://10.0.2.2:8000/
```

如果想选择目录、端口和并发参数，双击：

```text
start-static-dock-gui.bat
```

GUI 支持：

- 选择挂载目录
- 修改端口
- 修改 workers
- 修改 queue
- 启动/停止服务
- 打开本机地址
- 复制手机/模拟器访问地址

## Android 访问

真机和电脑在同一个 Wi-Fi 或手机连电脑热点时，使用电脑的局域网 IP：

```text
http://192.168.x.x:8000/loadtest/data/manifest.json
```

Android 模拟器使用：

```text
http://10.0.2.2:8000/loadtest/data/manifest.json
```

Android 需要网络权限：

```xml
<uses-permission android:name="android.permission.INTERNET" />
```

如果使用 `http://` 明文访问，Android 9+ 测试项目可临时开启：

```xml
<application android:usesCleartextTraffic="true">
</application>
```

## 命令行用法

```powershell
.\static-dock.exe --root "D:\android-data" --port 8000 --workers 128 --queue 2048
```

参数：

```text
--root PATH       要挂载的目录，默认是 exe 所在目录
--port PORT       监听端口，默认 8000；占用时会自动尝试备用端口
--workers N       工作线程数，默认按 CPU 自动取 64-128
--queue N         请求队列长度，默认至少 256
```

课堂建议：

```text
只发 JSON/图片：workers 32-64，queue 512
30-50 人同时访问：workers 96-128，queue 2048
多人看视频：workers 128，电脑最好网线接路由器
```

## 视频支持

StaticDock 支持普通 MP4 点播需要的能力：

- 流式发送文件，不会整文件读进内存
- `Accept-Ranges: bytes`
- `206 Partial Content`
- 播放器拖动进度条

它不是直播/转码服务器，不提供 RTMP、WebRTC、HLS 自动切片或自适应码率。

## 压测

仓库带有 `loadtest/` 示例数据，包含图片和 JSON。双击：

```text
run-loadtest.bat
```

脚本会临时启动服务、模拟多人并发打开页面、输出请求数、错误数、传输量和耗时，最后自动停止服务。

如果你把 MP4 文件放到：

```text
loadtest/videos/
```

压测脚本会自动加入视频 Range 和完整视频下载测试。

## 从源码构建

需要 Rust 工具链：

```powershell
cargo build --release
copy target\release\static-dock.exe static-dock.exe
```

也可以双击：

```text
build-windows.bat
```

## 注意

- 第一次启动时，Windows 防火墙可能询问是否允许访问网络；局域网使用需要允许“专用网络”。
- 真实课堂瓶颈通常是 Wi-Fi、电脑热点或路由器带宽，而不是 StaticDock 进程本身。
- 公开网络环境下不要挂载包含敏感文件的目录。

## License

MIT
