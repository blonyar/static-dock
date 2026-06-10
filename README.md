# StaticDock 静态资源码头

<p align="center">
  <img src="assets/showcase.png" alt="StaticDock 项目展示" width="520">
</p>

StaticDock 是一个轻量、可随身携带的本地静态资源服务器。它可以把任意文件夹一键挂载成 HTTP 服务，让浏览器、移动设备、局域网内其他电脑、模拟器或测试程序直接访问其中的图片、视频、JSON、文档等静态资源。

适合这些场景：

- 临时把本地文件夹变成可访问的 HTTP 资源目录。
- 给前端、移动端、桌面端或脚本程序提供测试资源。
- 在局域网、热点或离线环境中快速分发文件。
- 预览和分享图片、JSON、文档、音视频等静态内容。
- 不想安装 Tomcat、Node.js、Python，也不想配置复杂服务。

## 特性

- 单个 Windows 可执行文件：`static-dock.exe`
- 无运行时依赖：不需要 Node.js、Python、npm 包
- 支持目录挂载、端口配置、worker 线程数和请求队列
- 支持 HTTP Range：MP4 点播、拖动进度条可用
- 支持局域网、电脑热点、Android 模拟器访问
- 提供 Windows 桌面 GUI 启动器
- 提供并发压测脚本和测试素材
- Rust 实现，默认 128 workers / 2048 queue，适合临时共享和多端访问

## 快速开始

普通用户建议从 GitHub Releases 下载 `static-dock-windows-x64.zip`，解压后只需要双击：

```text
start-static-dock-gui.bat
```

它会打开原生图形界面 `StaticDockGui.exe`。默认挂载：

```text
resources/
```

打开服务后，默认访问的是资源管理页：

```text
http://127.0.0.1:8787/
```

把你的资源放进这些目录即可：

```text
resources/images/   图片、封面、头像、轮播图
resources/videos/   MP4/WebM 视频
resources/data/     JSON mock 数据
resources/docs/     文档或其他静态文件
```

GUI 支持：

- 选择挂载目录
- 修改端口、workers 和 queue
- 默认端口 `8787`，比 `8000/8080` 更不容易冲突
- 记住上次使用的目录、端口和最近使用目录下拉列表，配置保存在 `%APPDATA%\StaticDock\settings.ini`
- 自动过滤 `198.18.x.x` 等虚拟/测试网段，优先显示真实局域网地址
- 修改目录/端口时，如果服务正在运行，会自动重启生效
- 一键恢复默认设置
- 启动/停止服务
- 打开资源管理页
- 复制手机/模拟器/局域网访问地址
- 显示运行状态和服务日志

如果只想用命令行窗口直接启动，双击：

```text
start-static-dock.bat
```

它也会默认挂载 `resources/`，并使用端口 `8787`。

## 项目结构

```text
src/                 Rust 静态资源服务器源码
gui/                 C# WinForms GUI 源码
resources/           默认资源目录和浏览器资源管理台
package/windows/     Windows 发布包启动脚本来源
scripts/release/     release 自动化和包检查脚本
scripts/windows/     Windows fallback 脚本
tools/loadtest/      压测工具脚本
loadtest/            压测素材
docs/                架构和维护文档
```

更详细的架构说明见：[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)。

## Android / 模拟器访问示例

真机和电脑在同一个 Wi-Fi 或手机连电脑热点时，使用电脑的局域网 IP：

```text
http://192.168.x.x:8787/data/manifest.json
```

Android 模拟器使用：

```text
http://10.0.2.2:8787/data/manifest.json
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
.\static-dock.exe --root "D:\android-data" --port 8787 --workers 128 --queue 2048
```

参数：

```text
--root PATH       要挂载的目录，默认是 exe 所在目录
--port PORT       监听端口，默认 8787；占用时会自动尝试备用端口
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

仓库带有 `loadtest/` 示例数据，包含图片和 JSON。源码开发环境中运行：

```text
tools\loadtest\run-loadtest.bat
```

发布版 full 包根目录中可直接双击：

```text
run-loadtest.bat
```

脚本需要 PowerShell 7+（`pwsh.exe`），会临时启动服务、模拟多人并发打开页面、输出请求数、错误数、传输量和耗时，最后自动停止服务。

如果你把 MP4 文件放到：

```text
loadtest/videos/
```

压测脚本会自动加入视频 Range 和完整视频下载测试。

## 从源码构建

需要 Rust 工具链：

```powershell
cargo build --release
```

Windows 打包产物可通过一键脚本生成到 `build/windows/` 和 `dist/`，不会在仓库根目录生成 exe：

```text
build-windows.bat
```

## Release 打包

本项目提供一键发布脚本，会更新版本号、构建 Windows 包、运行 release 检查、提交、打 tag、推送并创建 GitHub Release。Release notes 默认优先读取 `CHANGELOG.md` 中对应版本，也可通过 `-NotesFile` 指定：

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/release/release.ps1 -Version v0.2.5
```

如只想本地演练构建和提交前检查，可先运行：

```powershell
.\build-windows.bat
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/release/test-release.ps1
```

## 注意

- 第一次启动时，Windows 防火墙可能询问是否允许访问网络；局域网使用需要允许“专用网络”。
- 真实课堂瓶颈通常是 Wi-Fi、电脑热点或路由器带宽，而不是 StaticDock 进程本身。
- 公开网络环境下不要挂载包含敏感文件的目录。

## License

MIT
