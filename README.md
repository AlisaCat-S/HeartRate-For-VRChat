# HeartRate for VRChat

这是一个通过蓝牙LE (Bluetooth Low Energy) 心率设备（如智能手环、手表）捕获实时心率，并通过 OSC (Open Sound Control) 协议将其发送到 VRChat 的工具。它还支持将心率数据实时写入本地文件，以便与其他软件（如直播推流工具 OBS）集成。

## ✨ 主要功能

-   **心率获取**：将心率数据发送到 VRChat，驱动 avatar 的心率动画或参数。
-   **选择设备**：
    -   启动时自动扫描并连接到附近的目标心率设备。
    -   三种连接模式（在 `config.toml` 中配置）：
        -   `auto`（默认）：优先匹配 `target_device_names` 中的设备名，无匹配时回退到信号最强的心率设备。
        -   `name`：仅按名称匹配。
        -   `strongest`：仅选择信号最强的心率设备（附近有他人的心率设备时可能连错，请留意程序打印的设备名）。
    -   内置心跳超时检测，当设备关机或断开连接时，程序会自动断开、清零状态并重新扫描连接。
-   **状态判断**：当检测到心率值为 `0`、设备断开、或程序退出时，会向 VRChat 发送 `false` 的连接状态并清零心率，使 avatar 能够表现出"未佩戴"状态，不会残留旧心率。
-   **文本文件输出（可选，默认关闭）**：在 `config.toml` 中将 `write_heart_rate_file` 设为 `true` 后，当前心率值会实时写入程序（exe）所在目录下的 `HeartRate.txt` 文件（仅在数值变化时写入，减少磁盘操作）。这使得其他软件可以轻松读取该文件，实现更多联动，例如在 OBS 直播画面上显示心率。断开或退出时该文件会被写为 `0`。

## 🔧 配置文件

首次运行时会在 exe 同目录自动生成 `config.toml`，可修改后重启程序生效：

| 配置项 | 默认值 | 说明 |
| --- | --- | --- |
| `selection_mode` | `"auto"` | 设备选择模式：`auto` / `name` / `strongest` |
| `target_device_names` | 小米/华为/荣耀等 | 名称匹配关键字列表（包含匹配） |
| `osc_ip` | `"127.0.0.1"` | OSC 目标 IP。Quest 一体机请改为头显的局域网 IP |
| `osc_port` | `9000` | OSC 目标端口。VRChat 用 `--osc` 改过端口的请同步修改 |
| `max_heart_rate_for_percent` | `200.0` | `hr_percent` 参数的分母 |
| `scan_duration_secs` | `5` | 每次扫描时长（秒） |
| `retry_delay_secs` | `5` | 断开后重试间隔（秒） |
| `heartbeat_timeout_secs` | `15` | 超过该秒数未收到心率数据则断开重连 |
| `write_heart_rate_file` | `false` | 是否将心率实时写入 `HeartRate.txt`（OBS 联动用，默认关闭以减少磁盘写入） |

## 📡 发送的 OSC 参数

程序向 VRChat 发送以下参数（且**仅有**以下参数）：

| OSC 地址 | 类型 | 取值 |
| --- | --- | --- |
| `/avatar/parameters/hr_connected` | Bool | 心率 > 0 时为 `true`，未佩戴/断开/退出时为 `false` |
| `/avatar/parameters/isHRActive` | Bool | 同上 |
| `/avatar/parameters/hr_percent` | Float | `心率 / max_heart_rate_for_percent`（默认 /200），范围 0.0–1.0 |
| `/avatar/parameters/VRCOSC/Heartrate/Normalised` | Float | `心率 / 240`，范围 0.0–1.0 |
| `/avatar/parameters/HR` | Int | 心率整数值（上限 240） |

> ⚠️ 兼容性说明：使用以上参数的预制件即可工作（下方列出了已测试的预制件）。
> 本程序**不发送** HRtoVRChat_OSC / Pulsoid 生态的 `onesHR`/`tensHR`/`hundredsHR`（逐位数字显示）、`floatHR`（(HR-127)/127）、`HeartRateInt`、`HeartBeatToggle` 等参数，依赖这些参数的预制件暂不支持。

## ⚙️ 支持的设备

已测试以下名称的设备：

-   Xiaomi Smart Band 9
-   Xiaomi Smart Band 10
-   Xiaomi Smart Band 10 Pro
-   HUAWEI (华为手环/手表)
-   HONOR (荣耀手环/手表)

理论上，任何遵循标准蓝牙 GATT 心率服务规范 (`0x180D`) 的设备都可以被支持（不限于上面的列表——该列表仅作为 `auto`/`name` 模式下的名称匹配关键字）。

## 🚀 如何使用

1.  从本项目的 **Releases** 页面下载最新的可执行文件。
2.  确保您的电脑已开启蓝牙功能。
3.  开启您的心率监测设备或功能，并确保它没有被其他设备 (`Pulsoid/码表`) 连接。
4.  执行下载的 `.exe` 文件（首次运行会生成 `config.toml`，一般无需修改）。
5.  程序将开始扫描设备。成功连接后，您会在终端窗口中看到实时的心率数据。请核对打印的设备名是否是您自己的设备。
6.  检查 VRChat，并确保已在 **菜单** 中启用了 OSC，并且 Avatar 使用上方参数表中的参数。

提示：VRChat 未启动时程序也可正常运行，会在 VRChat 启动后自动生效。

## 🪶 性能说明

本程序为游戏后台常驻设计：原生编译、单线程异步运行时，实测常驻内存约 8 MB、CPU 占用接近 0%，不会影响游戏性能。

唯一需要留意的潜在影响与程序本身无关，而在**无线电层面**：如果您的电脑蓝牙和 WiFi 集成在同一块 2.4GHz 无线芯片上（笔记本和常见台式机无线网卡均如此），蓝牙持续接收心率数据会轻微挤占 2.4GHz WiFi 的时隙，可能对 2.4G WiFi 的延迟产生细微影响。**使用网线或 5GHz WiFi 联网时完全不受影响**，推荐游戏时使用这两种联网方式。

## ⚙️ 已经测试过的的预制件
-   适配预制件1：https://booth.pm/ja/items/6224828
-   适配预制件2：https://booth.pm/ja/items/7197938

## 致谢
 
本项目的蓝牙心率读取功能，主要受到了 Tnze 开发的 [miband-heart-rate](https://github.com/Tnze/miband-heart-rate) 项目的启发，以及 西時流Behemoth 的帮助~
