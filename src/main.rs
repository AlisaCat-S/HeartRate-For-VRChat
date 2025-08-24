use std::io::{self, Write};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::time::Duration;
use std::{error, fmt, fs};

use futures_util::stream::StreamExt;
use tokio::time;
use uuid::Uuid;

use btleplug::api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Manager, Peripheral};

// --- 配置区 ---
struct Config {
    osc_ip: Ipv4Addr,
    osc_port: u16,
    target_device_names: &'static [&'static str],
    heart_rate_char_uuid: Uuid,
    max_heart_rate_for_percent: f32,
    scan_duration_secs: u64,
    retry_delay_secs: u64,
    heart_rate_service_uuid: Uuid,
    // --- 新增配置项 ---
    heartbeat_timeout_secs: u64, // 心跳超时时间（秒）
}

const CONFIG: Config = Config {
    osc_ip: Ipv4Addr::new(127, 0, 0, 1),
    osc_port: 9000,
    target_device_names: &[
        "Xiaomi Smart Band 9",
        "Xiaomi Smart Band 10",
        "HUAWEI",
        "HONOR",
    ],
    heart_rate_char_uuid: Uuid::from_u128(0x00002a37_0000_1000_8000_00805f9b34fb),
    heart_rate_service_uuid: Uuid::from_u128(0x0000180d_0000_1000_8000_00805f9b34fb),
    max_heart_rate_for_percent: 200.0,
    scan_duration_secs: 5,
    retry_delay_secs: 5,
    // --- 设置默认值 ---
    heartbeat_timeout_secs: 15, // 如果 15 秒没收到数据，就认为断线
};

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

// --- 文件输出 ---

/// 将心率写入程序目录下的 HeartRate.txt 文件。
/// 每次调用都会覆盖文件内容。
fn write_heart_rate_to_file(heart_rate: u8) -> io::Result<()> {
    // 使用 fs::write 可以简洁地实现文件的创建/覆盖和写入
    fs::write("HeartRate.txt", heart_rate.to_string())?;
    Ok(())
}

// --- OSC 通信 ---

/// 使用复用的 Socket 通过 OSC 格式化并发送心率数据。
/// - 使用 OSC Bundle 将四个消息合并到一个网络数据包中发送，以提高效率和数据同步性。
fn send_osc(socket: &UdpSocket, heart_rate: u8, config: &Config) -> Result<String> {
    // --- 【核心修改】 ---
    // 新增逻辑：判断心率是否为 0。
    // 如果心率大于 0，则认为设备已连接并处于活动状态。
    // 否则，视为未佩戴或无数据，is_active 为 false。
    let is_active = heart_rate > 0;

    // 1. 计算用于“百分比”的心率值
    let hr_for_percent = (heart_rate as f32).min(config.max_heart_rate_for_percent);
    let percent = hr_for_percent / config.max_heart_rate_for_percent;

    let hr_for_percent2 = (heart_rate as f32).min(240.0);
    let percent2 = hr_for_percent2 / 240.0;

    // 2. 准备用于“整数”的心率值
    let hr_for_int = heart_rate.min(240);

    // --- 将所有 OSC 消息打包到一个 Bundle 中 ---
    let bundle = rosc::OscPacket::Bundle(rosc::OscBundle {
        timetag: rosc::OscTime {
            seconds: 0,
            fractional: 1,
        },
        content: vec![
            // 消息 1: hr_connected
            rosc::OscPacket::Message(rosc::OscMessage {
                addr: "/avatar/parameters/hr_connected".to_string(),
                // --- 修改：使用 is_active 变量 ---
                args: vec![rosc::OscType::Bool(is_active)],
            }),
            // 消息 2: isHRActive
            rosc::OscPacket::Message(rosc::OscMessage {
                addr: "/avatar/parameters/isHRActive".to_string(),
                // --- 修改：使用 is_active 变量 ---
                args: vec![rosc::OscType::Bool(is_active)],
            }),
            // 消息 3: hr_percent (Float)
            rosc::OscPacket::Message(rosc::OscMessage {
                addr: "/avatar/parameters/hr_percent".to_string(),
                args: vec![rosc::OscType::Float(percent)],
            }),
            // 消息 3.5: hr_percent (Float)
            rosc::OscPacket::Message(rosc::OscMessage {
                addr: "/avatar/parameters/VRCOSC/Heartrate/Normalised".to_string(),
                args: vec![rosc::OscType::Float(percent2)],
            }),
            // 消息 4: HR (Int)
            rosc::OscPacket::Message(rosc::OscMessage {
                addr: "/avatar/parameters/HR".to_string(),
                args: vec![rosc::OscType::Int(hr_for_int as i32)],
            }),
        ],
    });

    // --- 编码并发送单个数据包 ---
    let buf = rosc::encoder::encode(&bundle)?;
    socket.send(&buf)?;

    // --- 修改：更新状态字符串以包含活动状态 ---
    Ok(format!(
        "心率: {} -> (OSC数据) -> Active: {}, Int: {}, Float/200: {:.2} %  Float2/240: {:.2} %",
        heart_rate, is_active, hr_for_int, percent, percent2
    ))
}

// --- 蓝牙逻辑 ---

/// 扫描并返回一个与外围设备。
async fn find_target_device(manager: &Manager, config: &Config) -> Result<Peripheral> {
    println!("正在扫描蓝牙设备...");
    let adapters = manager.adapters().await?;
    let central = adapters
        .into_iter()
        .next()
        .ok_or(AppError::AdapterNotFound)?;

    // 使用带有服务过滤的扫描
    let scan_filter = ScanFilter {
        services: vec![config.heart_rate_service_uuid],
    };
    // central.start_scan(ScanFilter::default()).await?;    // 扫描所有设备
    central.start_scan(scan_filter).await?; // 扫描包含心率服务的设备(可能无法获取设备名称)
    time::sleep(Duration::from_secs(config.scan_duration_secs)).await;

    // --- 1. 定义选择模式和配置 ---
    enum SelectionMode {
        ByName,
        StrongestSignal,
    }


    // *** 在这里切换模式 ***
    // let selection_mode = SelectionMode::StrongestSignal; //  SelectionMode::ByName
    let selection_mode = SelectionMode::StrongestSignal;


    // 当使用 ByName 模式时，这个列表会被用到
    let target_device_names = config.target_device_names;

    // --- 2. 扫描并处理设备 ---
    let peripherals = central.peripherals().await?;
    println!("附近设备列表:");

    if peripherals.is_empty() {
        println!("未发现任何设备。请检查设备是否开启并处于广播状态。");
    } else {
        // --- 3. 单次遍历，同时完成打印和寻找候选设备 ---
        let mut strongest_candidate: Option<(Peripheral, i16)> = None;
        let mut name_match_candidate: Option<Peripheral> = None;

        // 我们将使用一个循环来完成所有事情
        for p in peripherals {
            // 为了避免多次调用 .properties().await?，我们获取一次并复用
            // 如果获取不到属性，就跳过这个设备
            let properties = match p.properties().await? {
                Some(props) => props,
                None => continue,
            };

            // --- 打印逻辑 (和原来类似，稍作调整) ---
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
                // 过滤出 ASCII 字母和数字
                .filter(|c| c.is_ascii_alphanumeric())
                .collect();

            println!(
                "名称: {:<15} | MAC: {} | 信号强度: {}",
                // 同样，为了防止过滤后的名称过长，我们截取前15个字符
                filtered_device_name.chars().take(15).collect::<String>(),
                mac_address,
                rssi_str
            );

            // --- 候选设备选择逻辑 ---

            // 检查是否符合“按名称选择”的条件
            // 一旦找到第一个匹配项，就不会再更新 `name_match_candidate`
            if name_match_candidate.is_none() {
                if let Some(name) = &properties.local_name {
                    if target_device_names
                        .iter()
                        .any(|target| name.contains(target))
                    {
                        // peripheral `p` 在循环结束后会消失，所以我们需要克隆它来保留所有权
                        name_match_candidate = Some(p.clone());
                    }
                }
            }

            // 检查是否是“信号最强”的设备
            if let Some(rssi) = properties.rssi {
                // 如果 `strongest_candidate` 是空的，或者当前设备的信号更强
                if strongest_candidate.is_none() || rssi > strongest_candidate.as_ref().unwrap().1 {
                    // 更新最强者
                    strongest_candidate = Some((p.clone(), rssi));
                }
            }
        } // 循环结束

        // --- 根据配置模式，从候选者中选出最终设备 ---
        let chosen_peripheral = match selection_mode {
            SelectionMode::ByName => {
                println!("\n选择模式: 按名称查找, 关键字: {:?}", target_device_names);
                name_match_candidate
            }
            SelectionMode::StrongestSignal => {
                println!("\n选择模式: 选择信号最强的设备");
                // `strongest_candidate` 是一个元组 (Peripheral, i16)，我们只需要其中的 Peripheral
                strongest_candidate.map(|(p, _rssi)| p)
            }
        };

        // --- 处理最终结果 ---
        if let Some(p) = chosen_peripheral {
            // 再次获取属性以便打印最终选择的设备信息
            let props = p.properties().await?.unwrap_or_default();
            let name: String = props
                .local_name
                .unwrap_or("未知设备 Unknown Device".to_string());
            let filtered_device_name: String = name
                .chars()
                // 过滤出 ASCII 字母和数字
                .filter(|c| c.is_ascii_alphanumeric())
                .collect();
            println!("选择设备: {:?} ({})", filtered_device_name, p.address());

            central.stop_scan().await?;
            return Ok(p); // 返回找到的设备
        } else {
            println!("\n未找到符合条件的设备。");
        }
    }

    // 如果循环结束仍未找到设备，则返回错误。
    Err(AppError::DeviceNotFound)
}

/// 处理设备连接的整个生命周期。
async fn handle_device_connection(
    device: &Peripheral,
    socket: &UdpSocket,
    config: &Config,
) -> Result<()> {
    println!("\n正在连接设备 {}...", device.address());
    device.connect().await?;
    println!("设备连接成功！正在监听心率...");
    println!(
        "正在向 OSC 地址 {}:{} 发送数据",
        config.osc_ip, config.osc_port
    );

    device.discover_services().await?;

    let hr_char = device
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == config.heart_rate_char_uuid)
        .ok_or(AppError::CharacteristicNotFound)?;

    if !hr_char.properties.contains(CharPropFlags::NOTIFY) {
        eprintln!("错误：心率特征不支持通知 (Notify)。");
        return Err(AppError::SubscriptionFailed);
    }

    device.subscribe(&hr_char).await?;
    let mut notification_stream = device.notifications().await?;
    println!("已成功订阅心率通知。等待数据...");

    // --- 【核心修改】 ---
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
                break; // 跳出循环，触发重连
            }
            // Case 2: 成功接收到数据
            Ok(Some(notification)) => {
                if notification.uuid == config.heart_rate_char_uuid && notification.value.len() >= 2
                {
                    // 这里的代码和你原来的一样，用于解析和发送数据
                    let flag = notification.value[0];
                    let heart_rate: u16 = if (flag & 0x01) == 0 {
                        if notification.value.len() < 2 { continue; }
                        notification.value[1] as u16
                    } else {
                        if notification.value.len() < 3 { continue; }
                        u16::from_le_bytes([notification.value[1], notification.value[2]])
                    };

                    let heart_rate_u8 = heart_rate.min(255) as u8;

                    if let Err(e) = write_heart_rate_to_file(heart_rate_u8) {
                        eprintln!("\n写入心率到文件时出错: {}", e);
                    }

                    match send_osc(socket, heart_rate_u8, config) {
                        Ok(vrc_status) => {
                            print!("状态 -> {}   \r", vrc_status);
                            io::stdout().flush()?;
                        }
                        Err(e) => eprintln!("\n发送 OSC 数据时出错: {}", e),
                    }
                }
            }
            // Case 3: 通知流正常关闭 (例如设备主动优雅断连)
            Ok(None) => {
                println!("\n通知流已关闭。");
                break; // 同样跳出循环
            }
        }
    }

    Ok(())
}

// --- 主应用程序逻辑 ---
async fn main_loop(config: &'static Config) -> Result<()> {
    let manager = Manager::new().await?;

    // --- 优化：一次性创建 UDP 套接字并复用它。 ---
    let osc_addr = SocketAddrV4::new(config.osc_ip, config.osc_port);
    let socket = UdpSocket::bind("0.0.0.0:0")?; // 绑定到任何可用的本地端口
    socket.connect(osc_addr)?;
    println!("OSC Socket 已创建，将发送到 {}", osc_addr);

    loop {
        // 用于扫描的外部循环
        let device = match find_target_device(&manager, config).await {
            Ok(p) => p,
            Err(e) => {
                println!("\n错误: {}\n请检查设备是否在附近，电脑蓝牙是否开启。设备是否被其它心率接收设备连接。", e);
                println!("将在 {} 秒后重试扫描...", config.retry_delay_secs);
                time::sleep(Duration::from_secs(config.retry_delay_secs)).await;
                continue; // 重新开始扫描
            }
        };

        // 用于处理与已找到设备的连接的内部循环
        loop {
            // `is_connected` 检查有助于避免尝试连接到已连接的外围设备。
            // 在某些平台上，这可以防止突然断开连接后出错。
            if !device.is_connected().await? {
                if let Err(e) = handle_device_connection(&device, &socket, config).await {
                    eprintln!("\n处理连接时发生错误: {}", e);
                }
            }

            // 如果代码执行到这里，说明连接已断开或建立失败。
            println!(
                "\n连接已断开。将在 {} 秒后尝试重新连接...",
                config.retry_delay_secs
            );
            time::sleep(Duration::from_secs(config.retry_delay_secs)).await;

            // 在重试之前，检查设备是否仍被适配器“知晓”。
            // 如果不是，我们需要跳出并重新扫描。
            if manager
                .adapters()
                .await?
                .into_iter()
                .next()
                .ok_or(AppError::AdapterNotFound)?
                .peripherals()
                .await?
                .iter()
                .all(|p| p.address() != device.address())
            {
                println!("设备已从适配器列表中消失，将重新开始扫描...");
                break; // 跳出内部循环以重新扫描
            }
        }
    }
}

#[tokio::main]
async fn main() {
    println!("HeartRate For VRChat v{}", env!("CARGO_PKG_VERSION")); // 版本号更新
    println!("1.用于通过蓝牙接收心率广播数据，并发送至VRChat OSC");
    println!("支持设备: 小米手环 9/10、荣耀手环、华为手环/手表");
    println!("2.心率会同步输出到程序目录下的 HeartRate.txt 文件中,供其他软件使用");
    println!("适配预制件1：https://booth.pm/ja/items/6224828");
    println!("适配预制件2：https://booth.pm/ja/items/7197938");
    println!("PS:仅限能用————理论兼容所有Pulsoid适配的预制件。\nAuthor 箱天: 喵喵喵———— ");
    println!();

    if let Err(e) = main_loop(&CONFIG).await {
        eprintln!("\n发生错误: {}", e);
    }

    println!("\n程序已停止。");
}
