# HeartRate For VRChat

### 1.用于通过蓝牙接收心率广播数据，并发送至VRChat OSC

#### 支持设备: 小米手环 9/10、荣耀手环、华为手环/手表

### 2.心率会同步输出到程序目录下的 HeartRate.txt 文件中,供其他软件使用(如 OBS)

### 适配预制件1：https://booth.pm/ja/items/6224828

### 适配预制件2：https://booth.pm/ja/items/7197938

## Supported Platform

I use `bluest` crate. I copy its words below.

> Bluest is a cross-platform Bluetooth Low Energy (BLE) library for Rust. It currently supports Windows (version 10 and later), MacOS/iOS, and Linux. Android support is planned.

So it supported:

- Windows 10
- MacOS/iOS
- Linux

## 致谢
 
本项目的蓝牙心率读取功能，主要受到了 Tnze 开发的 [miband-heart-rate](https://github.com/Tnze/miband-heart-rate) 项目的启发，以及 西時流Behemoth 的帮助~

