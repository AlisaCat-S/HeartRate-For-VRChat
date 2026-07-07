use std::env;
use std::future::Future;
use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use std::{error, fmt, fs};

use futures_util::stream::StreamExt;
use serde::Deserialize;
use tokio::time;
use uuid::Uuid;

use btleplug::api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Manager, Peripheral};

// --- 蓝牙标准 UUID（固定值，无需配置） ---
const HEART_RATE_SERVICE_UUID: Uuid = Uuid::from_u128(0x0000180d_0000_1000_8000_00805f9b34fb);
const HEART_RATE_CHAR_UUID: Uuid = Uuid::from_u128(0x00002a37_0000_1000_8000_00805f9b34fb);

/// connect / discover_services 的超时（秒）：WinRT 上对不可达设备
/// 这些调用可能挂起数十秒甚至不返回，需要兜底。
const BLE_OP_TIMEOUT_SECS: u64 = 30;

/// 连续多少次连接失败（期间未收到任何心率数据）后放弃该设备、重新扫描。
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

// --- 配置（从 exe 同目录的 config.toml 加载，缺失时使用默认值并自动生成模板） ---
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct Config {
    /// 设备选择模式:
    /// "auto"      = 优先匹配 target_device_names，无匹配时回退到信号最强（默认）
    /// "name"      = 仅按名称匹配，找不到则重试扫描
    /// "strongest" = 仅选择信号最强的心率设备
    selection_mode: String,
    target_device_names: Vec<String>,
    osc_ip: String,
    osc_port: u16,
    max_heart_rate_for_percent: f32,
    scan_duration_secs: u64,
    retry_delay_secs: u64,
    /// 心跳超时时间（秒）：超过该时间未收到心率数据则重连
    heartbeat_timeout_secs: u64,
    /// 是否将心率实时写入 HeartRate.txt（供 OBS 等读取）
    write_heart_rate_file: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            selection_mode: "auto".to_string(),
            target_device_names: vec![
                "Xiaomi Smart Band 9".to_string(),
                "Xiaomi Smart Band 10".to_string(),
                "HUAWEI".to_string(),
                "HONOR".to_string(),
            ],
            osc_ip: "127.0.0.1".to_string(),
            osc_port: 9000,
            max_heart_rate_for_percent: 200.0,
            scan_duration_secs: 5,
            retry_delay_secs: 5,
            heartbeat_timeout_secs: 15,
            write_heart_rate_file: false,
        }
    }
}

const CONFIG_TEMPLATE: &str = r#"# HeartRate-For-VRChat 配置文件
# 删除本文件后重新运行程序可恢复默认配置。

# 设备选择模式:
#   "auto"      = 优先匹配 target_device_names 中的名称，无匹配时回退到信号最强（推荐）
#   "name"      = 仅按名称匹配，找不到则不断重试扫描
#   "strongest" = 仅选择信号最强的心率设备（附近有他人的心率设备时可能连错）
selection_mode = "auto"

# 按名称匹配时使用的设备名关键字（包含匹配）
target_device_names = [
    "Xiaomi Smart Band 9",
    "Xiaomi Smart Band 10",
    "HUAWEI",
    "HONOR",
]

# OSC 发送目标。本机 VRChat 保持默认即可；
# Quest 一体机请改为头显的局域网 IP；VRChat 用 --osc 改过端口的请同步修改。
osc_ip = "127.0.0.1"
osc_port = 9000

# hr_percent 参数的分母（心率/该值 = 百分比）
max_heart_rate_for_percent = 200.0

# 每次扫描时长（秒）
scan_duration_secs = 5

# 断开后重试间隔（秒）
retry_delay_secs = 5

# 心跳超时时间（秒）：超过该时间未收到心率数据则断开重连
heartbeat_timeout_secs = 15

# 是否将心率实时写入程序目录下的 HeartRate.txt（供 OBS 等其他软件读取）。
# 默认关闭以减少磁盘写入；需要 OBS 联动时改为 true。
write_heart_rate_file = false
"#;

/// 获取 exe 所在目录；失败时回退到当前工作目录（绝对路径）。
fn exe_dir() -> PathBuf {
    match env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        Some(dir) => dir,
        None => {
            let fallback = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            eprintln!(
                "警告：无法获取 exe 所在目录，配置和 HeartRate.txt 将使用当前目录: {}",
                fallback.display()
            );
            fallback
        }
    }
}

/// 从 exe 同目录加载 config.toml；文件不存在则生成模板并返回默认配置。
/// 加载后对取值做合法性校验/钳制。
fn load_config(dir: &Path) -> Config {
    let path = dir.join("config.toml");
    let mut config = match fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(config) => {
                println!("已加载配置文件: {}", path.display());
                config
            }
            Err(e) => {
                eprintln!("=============================================");
                eprintln!("警告：配置文件解析失败，本次运行将忽略其中的【全部】设置，使用默认配置！");
                eprintln!("文件: {}", path.display());
                eprintln!("原因: {}", e);
                eprintln!("请修正后重启程序（或删除该文件以重新生成模板）。");
                eprintln!("=============================================");
                Config::default()
            }
        },
        Err(_) => {
            match fs::write(&path, CONFIG_TEMPLATE) {
                Ok(()) => println!("已生成默认配置文件: {}（可编辑后重启程序生效）", path.display()),
                Err(e) => eprintln!("无法生成配置文件 {}: {}，将使用默认配置。", path.display(), e),
            }
            Config::default()
        }
    };

    // 校验 selection_mode，非法值回退 auto 并给出明确提示
    let mode = config.selection_mode.trim().to_ascii_lowercase();
    if matches!(mode.as_str(), "auto" | "name" | "strongest") {
        config.selection_mode = mode;
    } else {
        eprintln!(
            "警告：selection_mode = \"{}\" 不是有效值（auto / name / strongest），将按 auto 处理。",
            config.selection_mode
        );
        config.selection_mode = "auto".to_string();
    }

    // 数值下限钳制，避免 0 值导致扫描不到设备或连接后立即超时
    if config.heartbeat_timeout_secs < 3 {
        eprintln!("警告：heartbeat_timeout_secs 过小，已调整为 3。");
        config.heartbeat_timeout_secs = 3;
    }
    if config.scan_duration_secs < 1 {
        eprintln!("警告：scan_duration_secs 过小，已调整为 1。");
        config.scan_duration_secs = 1;
    }
    if config.retry_delay_secs < 1 {
        eprintln!("警告：retry_delay_secs 过小，已调整为 1。");
        config.retry_delay_secs = 1;
    }
    if config.max_heart_rate_for_percent < 1.0 {
        eprintln!("警告：max_heart_rate_for_percent 过小，已调整为 200。");
        config.max_heart_rate_for_percent = 200.0;
    }

    config
}

// --- 自定义错误类型 ---
#[derive(Debug)]
enum AppError {
    Btleplug(btleplug::Error),
    Io(io::Error),
    Rosc(rosc::OscError),
    AdapterNotFound,
    DeviceNotFound,
    CharacteristicNotFound,
    SubscriptionFailed,
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Btleplug(e) => write!(f, "蓝牙错误: {}", e),
            AppError::Io(e) => write!(f, "I/O 错误: {}", e),
            AppError::Rosc(e) => write!(f, "OSC 编码错误: {}", e),
            AppError::AdapterNotFound => write!(f, "未找到蓝牙适配器。"),
            AppError::DeviceNotFound => write!(f, "未能找到目标设备。"),
            AppError::CharacteristicNotFound => write!(f, "未找到心率特征。"),
            AppError::SubscriptionFailed => write!(f, "订阅通知失败。"),
        }
    }
}

impl error::Error for AppError {}

// --- 转换器，以便可以使用 `?` 运算符 ---
impl From<btleplug::Error> for AppError {
    fn from(e: btleplug::Error) -> Self {
        AppError::Btleplug(e)
    }
}
impl From<io::Error> for AppError {
    fn from(e: io::Error) -> Self {
        AppError::Io(e)
    }
}
impl From<rosc::OscError> for AppError {
    fn from(e: rosc::OscError) -> Self {
        AppError::Rosc(e)
    }
}

type Result<T> = std::result::Result<T, AppError>;

/// 为可能挂起的 BLE 操作加超时兜底。
async fn ble_timeout<F, T>(fut: F) -> Result<T>
where
    F: Future<Output = btleplug::Result<T>>,
{
    let dur = Duration::from_secs(BLE_OP_TIMEOUT_SECS);
    match time::timeout(dur, fut).await {
        Ok(r) => Ok(r?),
        Err(_) => Err(AppError::Btleplug(btleplug::Error::TimedOut(dur))),
    }
}

// --- OSC 通信 ---

/// 通过 OSC 格式化并发送心率数据。
/// 使用 OSC Bundle 将所有消息合并到一个网络数据包中发送。
/// Windows 上目标端口无人监听（VRChat 未启动）时 UDP 可能返回
/// WSAECONNRESET(10054)——这只表示"对端没人听"，视为已发送。
fn send_osc(
    socket: &UdpSocket,
    osc_addr: SocketAddrV4,
    heart_rate: u8,
    config: &Config,
) -> Result<String> {
    // 心率大于 0 视为已佩戴/有数据；0 视为未佩戴或已断开。
    let is_active = heart_rate > 0;

    let max_hr = config.max_heart_rate_for_percent.max(1.0);
    let percent = (heart_rate as f32).min(max_hr) / max_hr;

    let percent2 = (heart_rate as f32).min(240.0) / 240.0;

    let hr_for_int = heart_rate.min(240);

    let bundle = rosc::OscPacket::Bundle(rosc::OscBundle {
        // {0, 1} 是 OSC 规范中的 "immediately"
        timetag: rosc::OscTime {
            seconds: 0,
            fractional: 1,
        },
        content: vec![
            rosc::OscPacket::Message(rosc::OscMessage {
                addr: "/avatar/parameters/hr_connected".to_string(),
                args: vec![rosc::OscType::Bool(is_active)],
            }),
            rosc::OscPacket::Message(rosc::OscMessage {
                addr: "/avatar/parameters/isHRActive".to_string(),
                args: vec![rosc::OscType::Bool(is_active)],
            }),
            rosc::OscPacket::Message(rosc::OscMessage {
                addr: "/avatar/parameters/hr_percent".to_string(),
                args: vec![rosc::OscType::Float(percent)],
            }),
            rosc::OscPacket::Message(rosc::OscMessage {
                addr: "/avatar/parameters/VRCOSC/Heartrate/Normalised".to_string(),
                args: vec![rosc::OscType::Float(percent2)],
            }),
            rosc::OscPacket::Message(rosc::OscMessage {
                addr: "/avatar/parameters/HR".to_string(),
                args: vec![rosc::OscType::Int(hr_for_int as i32)],
            }),
        ],
    });

    let buf = rosc::encoder::encode(&bundle)?;
    if let Err(e) = socket.send_to(&buf, osc_addr) {
        if e.kind() != io::ErrorKind::ConnectionReset {
            return Err(e.into());
        }
    }

    Ok(format!(
        "心率: {} -> (OSC数据) -> Active: {}, Int: {}, Float/{}: {:.2}  Float/240: {:.2}",
        heart_rate, is_active, hr_for_int, max_hr, percent, percent2
    ))
}

/// 断开/退出时向 VRChat 发送清零状态（is_active=false, HR=0），
/// 若启用了文件输出则把 HeartRate.txt 写为 0，避免 avatar 和 OBS 残留旧心率。
fn clear_state(socket: &UdpSocket, osc_addr: SocketAddrV4, config: &Config, hr_file: &Path) {
    let _ = send_osc(socket, osc_addr, 0, config);
    if config.write_heart_rate_file {
        let _ = fs::write(hr_file, "0");
    }
}

// --- 退出清理（Ctrl-C / 关闭窗口 / 注销 / 关机） ---

struct CleanupCtx {
    osc_addr: SocketAddrV4,
    config: Config,
    hr_file: PathBuf,
}

static CLEANUP_CTX: OnceLock<CleanupCtx> = OnceLock::new();
static CLEANUP_DONE: AtomicBool = AtomicBool::new(false);

/// 只执行一次的退出清理。
fn run_exit_cleanup() {
    if CLEANUP_DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(ctx) = CLEANUP_CTX.get() {
        match UdpSocket::bind("0.0.0.0:0") {
            Ok(socket) => clear_state(&socket, ctx.osc_addr, &ctx.config, &ctx.hr_file),
            Err(_) => {
                if ctx.config.write_heart_rate_file {
                    let _ = fs::write(&ctx.hr_file, "0");
                }
            }
        }
    }
}

/// 注册控制台事件处理器。
/// 不使用 ctrlc crate：它的处理例程立即返回、闭包在另一线程异步执行，
/// CTRL_CLOSE_EVENT（点 X 关窗）下会与进程终止竞争，清理大概率来不及跑。
/// 这里直接在处理例程内【同步】完成清理再返回（CLOSE 事件下例程有约 5 秒预算）。
#[cfg(windows)]
fn register_exit_handler() -> bool {
    use windows_sys::Win32::System::Console::{
        SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT,
    };

    unsafe extern "system" fn handler(ctrl_type: u32) -> i32 {
        run_exit_cleanup();
        match ctrl_type {
            // Ctrl-C / Ctrl-Break：系统不主动终止进程，由我们自己退出
            CTRL_C_EVENT | CTRL_BREAK_EVENT => std::process::exit(0),
            // CLOSE / LOGOFF / SHUTDOWN：返回 FALSE，交给系统继续默认终止流程
            _ => 0,
        }
    }

    unsafe { SetConsoleCtrlHandler(Some(handler), 1) != 0 }
}

#[cfg(not(windows))]
fn register_exit_handler() -> bool {
    false
}

// --- 蓝牙逻辑 ---

/// 扫描并返回一个目标外围设备。
async fn find_target_device(manager: &Manager, config: &Config) -> Result<Peripheral> {
    println!("正在扫描蓝牙设备...");
    let adapters = manager.adapters().await?;
    let central = adapters
        .into_iter()
        .next()
        .ok_or(AppError::AdapterNotFound)?;

    // 只扫描广播了心率服务 (0x180D) 的设备
    let scan_filter = ScanFilter {
        services: vec![HEART_RATE_SERVICE_UUID],
    };
    central.start_scan(scan_filter).await?;
    time::sleep(Duration::from_secs(config.scan_duration_secs)).await;

    let peripherals = central.peripherals().await?;
    println!("附近设备列表:");

    let mut strongest_candidate: Option<(Peripheral, i16)> = None;
    let mut name_match_candidate: Option<Peripheral> = None;

    if peripherals.is_empty() {
        println!("未发现任何设备。请检查设备是否开启并处于广播状态。");
    }

    for p in peripherals {
        // 获取不到属性的设备直接跳过
        let properties = match p.properties().await {
            Ok(Some(props)) => props,
            _ => continue,
        };

        let mac_address = p.address();
        let device_name = properties
            .local_name
            .clone()
            .unwrap_or_else(|| "未知设备 Unknown Device".to_string());
        let rssi_str = properties
            .rssi
            .map_or("N/A".to_string(), |rssi| format!("{} dBm", rssi));

        let filtered_device_name: String = device_name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();

        println!(
            "名称: {:<15} | MAC: {} | 信号强度: {}",
            filtered_device_name.chars().take(15).collect::<String>(),
            mac_address,
            rssi_str
        );

        // 名称匹配候选（保留第一个匹配项）
        if name_match_candidate.is_none() {
            if let Some(name) = &properties.local_name {
                if config
                    .target_device_names
                    .iter()
                    .any(|target| name.contains(target.as_str()))
                {
                    name_match_candidate = Some(p.clone());
                }
            }
        }

        // 信号最强候选
        if let Some(rssi) = properties.rssi {
            if strongest_candidate
                .as_ref()
                .is_none_or(|(_, best)| rssi > *best)
            {
                strongest_candidate = Some((p.clone(), rssi));
            }
        }
    }

    let chosen_peripheral = match config.selection_mode.as_str() {
        "name" => {
            println!(
                "\n选择模式: 按名称匹配, 关键字: {:?}",
                config.target_device_names
            );
            name_match_candidate
        }
        "strongest" => {
            println!("\n选择模式: 选择信号最强的设备");
            strongest_candidate.map(|(p, _rssi)| p)
        }
        _ => {
            println!(
                "\n选择模式: 自动（优先匹配名称 {:?}，无匹配时选择信号最强）",
                config.target_device_names
            );
            name_match_candidate.or(strongest_candidate.map(|(p, _rssi)| p))
        }
    };

    // 无论成功与否都停止扫描
    let _ = central.stop_scan().await;

    match chosen_peripheral {
        Some(p) => {
            let props = p.properties().await?.unwrap_or_default();
            let name = props
                .local_name
                .unwrap_or_else(|| "未知设备 Unknown Device".to_string());
            let filtered_device_name: String = name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect();
            println!("选择设备: {:?} ({})", filtered_device_name, p.address());
            Ok(p)
        }
        None => {
            println!("\n未找到符合条件的设备。");
            Err(AppError::DeviceNotFound)
        }
    }
}

/// 处理设备连接的整个生命周期。
/// 返回 Ok(true) 表示本次连接期间至少收到过一次心率数据；
/// 断开清理（disconnect）由调用方统一执行。
async fn handle_device_connection(
    device: &Peripheral,
    socket: &UdpSocket,
    osc_addr: SocketAddrV4,
    config: &Config,
    hr_file: &Path,
) -> Result<bool> {
    // is_connected 查询失败时视为未连接，直接尝试 connect
    if !device.is_connected().await.unwrap_or(false) {
        println!("\n正在连接设备 {}...", device.address());
        ble_timeout(device.connect()).await?;
    }
    println!("设备连接成功！正在监听心率...");
    println!("正在向 OSC 地址 {} 发送数据", osc_addr);

    ble_timeout(device.discover_services()).await?;

    let hr_char = device
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == HEART_RATE_CHAR_UUID)
        .ok_or(AppError::CharacteristicNotFound)?;

    // Notify 和 Indicate 都可以订阅（btleplug 会自动选择正确的 CCCD 值）
    if !hr_char
        .properties
        .intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE)
    {
        eprintln!("错误：心率特征不支持通知 (Notify/Indicate)。");
        return Err(AppError::SubscriptionFailed);
    }

    ble_timeout(device.subscribe(&hr_char)).await?;
    let mut notification_stream = device.notifications().await?;
    println!("已成功订阅心率通知。等待数据...");

    let mut received_any = false;
    // 心率数值变化时才写文件：fs::write 每次都是完整的打开/截断/写/关闭，
    // 还可能触发杀毒软件实时扫描，是本程序最重的单个动作
    let mut last_written_hr: Option<u8> = None;
    // 错误只提示一次，恢复后重置（避免 VRChat 未启动时每秒刷屏）
    let mut osc_error_shown = false;
    let mut file_error_shown = false;

    // 使用 `loop` 和 `tokio::time::timeout` 来实现带超时的事件接收
    loop {
        match time::timeout(
            Duration::from_secs(config.heartbeat_timeout_secs),
            notification_stream.next(),
        )
        .await
        {
            // Case 1: 超时发生
            Err(_) => {
                println!(
                    "\n未在 {} 秒内收到心率数据，认为连接已断开。",
                    config.heartbeat_timeout_secs
                );
                break;
            }
            // Case 2: 成功接收到数据
            Ok(Some(notification)) => {
                if notification.uuid == HEART_RATE_CHAR_UUID && notification.value.len() >= 2 {
                    // 解析 GATT Heart Rate Measurement: flags 位 0 决定 8/16 位格式
                    let flag = notification.value[0];
                    let heart_rate: u16 = if (flag & 0x01) == 0 {
                        notification.value[1] as u16
                    } else {
                        if notification.value.len() < 3 {
                            continue;
                        }
                        u16::from_le_bytes([notification.value[1], notification.value[2]])
                    };

                    received_any = true;
                    let heart_rate_u8 = heart_rate.min(255) as u8;

                    if config.write_heart_rate_file && last_written_hr != Some(heart_rate_u8) {
                        match fs::write(hr_file, heart_rate_u8.to_string()) {
                            Ok(()) => {
                                last_written_hr = Some(heart_rate_u8);
                                file_error_shown = false;
                            }
                            Err(e) => {
                                last_written_hr = None;
                                if !file_error_shown {
                                    eprintln!(
                                        "\n写入心率到文件 {} 时出错: {}（恢复前不再重复提示）",
                                        hr_file.display(),
                                        e
                                    );
                                    file_error_shown = true;
                                }
                            }
                        }
                    }

                    match send_osc(socket, osc_addr, heart_rate_u8, config) {
                        Ok(vrc_status) => {
                            osc_error_shown = false;
                            print!("状态 -> {}   \r", vrc_status);
                            let _ = io::stdout().flush();
                        }
                        Err(e) => {
                            if !osc_error_shown {
                                eprintln!("\n发送 OSC 数据时出错: {}（将继续重试，恢复前不再重复提示）", e);
                                osc_error_shown = true;
                            }
                        }
                    }
                }
            }
            // Case 3: 通知流正常关闭 (例如设备主动优雅断连)
            Ok(None) => {
                println!("\n通知流已关闭。");
                break;
            }
        }
    }

    // 释放订阅；断开连接由调用方统一处理
    let _ = device.unsubscribe(&hr_char).await;
    Ok(received_any)
}

// --- 主应用程序逻辑 ---
async fn main_loop(config: &Config, osc_addr: SocketAddrV4, hr_file: &Path) -> Result<()> {
    let manager = Manager::new().await?;

    // 一次性创建 UDP 套接字并复用（用 send_to 发送）
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    println!("OSC Socket 已创建，将发送到 {}", osc_addr);

    loop {
        // 用于扫描的外部循环
        let device = match find_target_device(&manager, config).await {
            Ok(p) => p,
            Err(e) => {
                println!("\n错误: {}\n请检查设备是否在附近，电脑蓝牙是否开启。设备是否被其它心率接收设备连接。", e);
                println!("将在 {} 秒后重试扫描...", config.retry_delay_secs);
                time::sleep(Duration::from_secs(config.retry_delay_secs)).await;
                continue;
            }
        };

        // 内部循环：对同一设备重试连接。
        // 连续 MAX_CONSECUTIVE_FAILURES 次未收到任何心率数据则放弃该设备、重新扫描
        // （设备可能已关机/走远/更换了随机 MAC 地址）。
        let mut consecutive_failures: u32 = 0;
        loop {
            let received_any =
                match handle_device_connection(&device, &socket, osc_addr, config, hr_file).await {
                    Ok(received) => received,
                    Err(e) => {
                        eprintln!("\n处理连接时发生错误: {}", e);
                        false
                    }
                };

            // 无论因超时、流关闭还是错误退出，都显式断开连接，
            // 确保下一轮能重新走完整的 connect/subscribe 流程，
            // 避免链路残留导致"看似在重连、实际永不重订阅"的死循环。
            let _ = device.disconnect().await;

            // 断开期间向 VRChat 发送未佩戴状态，并清零 HeartRate.txt
            clear_state(&socket, osc_addr, config, hr_file);

            if received_any {
                consecutive_failures = 0;
            } else {
                consecutive_failures += 1;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    println!(
                        "\n连续 {} 次未能从设备获取心率数据，将重新开始扫描...",
                        consecutive_failures
                    );
                    break;
                }
            }

            println!(
                "\n连接已断开。将在 {} 秒后尝试重新连接...",
                config.retry_delay_secs
            );
            time::sleep(Duration::from_secs(config.retry_delay_secs)).await;
        }
    }
}

/// 出错退出前暂停，避免双击运行时窗口一闪而过看不到错误信息。
fn pause_before_exit() {
    println!("按回车键退出...");
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
}

// 单线程运行时足矣：本程序每秒只处理一条蓝牙通知，
// 默认的多线程运行时会按 CPU 核数起 worker 线程，纯属浪费。
#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("HeartRate For VRChat v{}", env!("CARGO_PKG_VERSION"));
    println!("1.通过蓝牙连接心率设备（任何标准 GATT 心率服务 0x180D 设备），将心率发送至 VRChat OSC");
    println!("2.可选：在 config.toml 中开启 write_heart_rate_file 后，心率会同步写入程序目录下的 HeartRate.txt（供 OBS 等软件使用，默认关闭）");
    println!("3.连接模式、设备名、OSC 地址等可在程序目录下的 config.toml 中修改");
    println!("发送的 OSC 参数列表见 README（hr_connected / isHRActive / hr_percent / VRCOSC Normalised / HR）");
    println!("适配预制件1：https://booth.pm/ja/items/6224828");
    println!("适配预制件2：https://booth.pm/ja/items/7197938");
    println!("Author 箱天: 喵喵喵———— ");
    println!();

    let dir = exe_dir();
    let config = load_config(&dir);
    let hr_file = dir.join("HeartRate.txt");

    let osc_ip: Ipv4Addr = match config.osc_ip.parse() {
        Ok(ip) => ip,
        Err(_) => {
            eprintln!(
                "配置中的 osc_ip \"{}\" 不是有效的 IPv4 地址，将使用 127.0.0.1。",
                config.osc_ip
            );
            Ipv4Addr::LOCALHOST
        }
    };
    let osc_addr = SocketAddrV4::new(osc_ip, config.osc_port);

    // 注册退出清理（Ctrl-C / 点 X 关窗 / 注销 / 关机时向 VRChat 发送清零状态）
    let _ = CLEANUP_CTX.set(CleanupCtx {
        osc_addr,
        config: config.clone(),
        hr_file: hr_file.clone(),
    });
    if !register_exit_handler() {
        eprintln!("注册退出清理处理器失败（退出时 VRChat 可能残留最后一次心率）。");
    }

    if let Err(e) = main_loop(&config, osc_addr, &hr_file).await {
        eprintln!("\n发生错误: {}", e);
        eprintln!("请检查电脑是否有蓝牙适配器、蓝牙服务是否已启动。");
        pause_before_exit();
        return;
    }

    println!("\n程序已停止。");
}
