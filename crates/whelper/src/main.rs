// Release 构建使用 GUI 子系统，避免常驻程序弹出控制台；Debug 构建保留终端日志。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

//! whelper：单一可执行文件、单一进程，同时运行三个桌面组件与一个托盘外壳。
//!
//! - `text-picker`（划词工具条）：创建工具条窗口并跑主消息循环，进程生命周期由它决定；
//! - `show-desktop`（单屏显示桌面）：在同一线程创建消息窗口，把自己注册到共享键盘钩子链，
//!   由同一消息循环驱动；触发时只吞掉 `Win+D`；
//! - `window-switcher`（Alt+Tab 切换器）：在同一线程创建覆盖窗口，注册 Alt+Tab 处理器；
//!   缩略图/网格两种视图与焦点切换都在覆盖窗口里完成；
//! - `whelper-shell`（桌面外壳）：托盘图标、菜单、设置页与开机自启；设置页里的开关
//!   写回 `%APPDATA%\whelper\config.ini`，三个组件在事件/定时器里读到新值。
//!
//! 启动顺序：先装载配置，再启动三个服务（各建一个窗口 + 注册钩子，毫秒级），
//! 最后进入工具条窗口的消息循环。所有窗口共用主线程与同一条键盘钩子链。

use whelper_common::win32::instance::SingleInstance;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        help();
        return;
    }

    // DPI 感知必须在任何窗口之前：托盘/设置窗口可能先于工具条创建。
    whelper_common::win32::dpi::enable_per_monitor_v2();

    // 配置要先于任何组件装载：设置页、钩子、绘制都会读它。
    whelper_config::install(whelper_config::load());

    // 一次性命令：做完即退出，不进入常驻模式，也不参与单实例互斥。
    if args.iter().any(|arg| arg == "--self-test") {
        std::process::exit(text_picker::run_self_test());
    }
    if let Some(index) = args.iter().position(|arg| arg == "--translate") {
        let text = args.get(index + 1).cloned().unwrap_or_default();
        if text.trim().is_empty() {
            eprintln!("用法: whelper --translate <text>");
            std::process::exit(2);
        }
        std::process::exit(match text_picker::translate_blocking(&text) {
            Ok(result) => {
                println!("{result}");
                0
            }
            Err(error) => {
                eprintln!("翻译失败: {error}");
                1
            }
        });
    }
    if args.iter().any(|arg| arg == "--capture-probe") {
        std::process::exit(text_picker::probe_capture());
    }
    if args.iter().any(|arg| arg == "--desktop-report") {
        std::process::exit(show_desktop::app::report());
    }
    if args.iter().any(|arg| arg == "--toggle-desktop") {
        std::process::exit(show_desktop::app::toggle_once());
    }
    if args.iter().any(|arg| arg == "--desktop-self-test") {
        std::process::exit(show_desktop::app::self_test());
    }
    if args.iter().any(|arg| arg == "--switcher-report") {
        std::process::exit(window_switcher::app::report());
    }
    if args.iter().any(|arg| arg == "--switcher-self-test") {
        std::process::exit(window_switcher::app::self_test());
    }
    if args.iter().any(|arg| arg == "--switcher-demo") {
        std::process::exit(window_switcher::app::demo());
    }
    if args.iter().any(|arg| arg == "--icon-report") {
        std::process::exit(whelper_shell::icon_report());
    }
    if args.iter().any(|arg| arg == "--config-report") {
        config_report();
        return;
    }

    let demo = args.iter().any(|arg| arg == "--demo");
    let open_settings = args.iter().any(|arg| arg == "--settings");
    let autostart = args.iter().any(|arg| arg == "--autostart");
    if autostart {
        // 开机自启时不要弹控制台窗口；尽早藏起来，减少闪一下。
        whelper_common::win32::console::hide_console();
    }

    // 一个进程一个实例：四个模块共享同一个互斥体，避免钩子/托盘被重复安装。
    let _guard = match SingleInstance::acquire("Local\\WHelper_SingleInstance") {
        Some(guard) => guard,
        None => {
            if open_settings {
                if !whelper_shell::open_settings_in_running_instance() {
                    eprintln!("whelper 已在运行，但设置页没有响应");
                }
            } else {
                eprintln!("whelper 已在运行");
            }
            std::process::exit(0);
        }
    };

    // 先注册桌面服务：它的处理器排在钩子链前面，Win+D 不会再落到系统手里。
    let desktop = if demo {
        None
    } else {
        show_desktop::service::Service::start()
    };

    // 切窗服务排在其后：Alt+Tab 与 Win+D 互不干扰，钩子链按注册顺序依次询问。
    let switcher = if demo {
        None
    } else {
        window_switcher::service::Service::start()
    };

    // 托盘外壳：隐藏窗口 + 托盘图标；设置页按需创建。
    let mut shell = if demo {
        None
    } else {
        whelper_shell::Shell::start()
    };

    if !demo && !autostart {
        println!("[whelper] 配置文件：{}", whelper_config::config_path().display());
        match desktop.as_ref() {
            Some(service) => println!(
                "[whelper] 单屏显示桌面：已接管 Win+D（消息窗口 {:#x}）",
                service.hwnd().0 as usize
            ),
            None => eprintln!("[warn] 单屏显示桌面未启用：消息窗口或键盘钩子创建失败"),
        }
        match switcher.as_ref() {
            Some(service) => println!(
                "[whelper] Alt+Tab 切换器：已接管 Alt+Tab（覆盖窗口 {:#x}）",
                service.hwnd().0 as usize
            ),
            None => eprintln!("[warn] Alt+Tab 切换器未启用：覆盖窗口或键盘钩子创建失败"),
        }
        match shell.as_ref() {
            Some(shell) => println!(
                "[whelper] 托盘已就绪（外壳窗口 {:#x}），左键打开设置",
                shell.hwnd().0 as usize
            ),
            None => eprintln!("[warn] 托盘图标创建失败：外壳窗口未创建"),
        }
        println!(
            "[whelper] pid={} 进程内组件：划词工具条 + 单屏显示桌面 + Alt+Tab 切换器 + 托盘设置",
            std::process::id()
        );
    }

    if open_settings {
        if let Some(shell) = shell.as_mut() {
            shell.open_settings();
        }
    }

    // 主消息循环（工具条窗口）也负责派发桌面服务、切窗服务与外壳/设置窗口的消息。
    let code = text_picker::run_ui(demo);

    // 先撤托盘、设置页，再销毁两个服务（摘钩子、销毁窗口），最后退出进程。
    drop(shell);
    drop(switcher);
    drop(desktop);
    std::process::exit(code);
}

/// `--config-report`：打印当前生效的配置（内存中的值 + 文件路径）。
fn config_report() {
    let path = whelper_config::config_path();
    let config = whelper_config::get();
    println!(
        "配置文件：{}（{}）",
        path.display(),
        if path.exists() { "已存在" } else { "尚未创建" }
    );
    println!("主题：{}　调试日志：{}", config.general.theme.as_str(), config.general.debug);
    println!(
        "划词工具条：{}（双击选词 {}，拖拽阈值 {}px）",
        on_off(config.picker.enabled),
        on_off(config.picker.double_click),
        config.picker.drag_threshold
    );
    println!("单屏显示桌面：{}", on_off(config.desktop.enabled));
    println!(
        "Alt+Tab 切换器：{}（默认视图 {}，窗口预览 {}，过渡动画 {}）",
        on_off(config.switcher.enabled),
        config.switcher.default_view.as_str(),
        on_off(config.switcher.previews),
        on_off(config.switcher.animation)
    );
}

fn on_off(value: bool) -> &'static str {
    if value {
        "开启"
    } else {
        "关闭"
    }
}

fn help() {
    println!("whelper [--settings | --autostart | --demo | --self-test | --translate <text>]");
    println!("        [--desktop-report | --toggle-desktop | --desktop-self-test]");
    println!("        [--switcher-report | --switcher-self-test | --switcher-demo]");
    println!("        [--icon-report | --config-report]");
    println!("  (无参数)             常驻：划词工具条 + 单屏显示桌面 + Alt+Tab 切换器 + 托盘");
    println!("  --settings           打开设置页；已在运行时把现有实例的设置页拉到前台");
    println!("  --autostart          开机自启入口：隐藏控制台后常驻（注册表启动项使用）");
    println!("  --demo               只演示工具条 UI（不装钩子、不建托盘）");
    println!("  --self-test          取词 + 剪贴板还原自检");
    println!("  --capture-probe      只读取词诊断：打印当前选区与是否动过剪贴板");
    println!("  --translate <text>   直接调用翻译接口并打印结果（调试用）");
    println!("  --desktop-report     只读预览：光标显示器的候选窗口与下一步动作");
    println!("  --toggle-desktop     立即执行一次单屏显示桌面");
    println!("  --desktop-self-test  单屏显示桌面：最小化→还原往返自检");
    println!("  --switcher-report    只读预览：Alt+Tab 候选窗口、预览抓取与两种视图尺寸");
    println!("  --switcher-self-test Alt+Tab 切换器：激活另一个窗口→校验前台→切回");
    println!("  --switcher-demo      接管 Alt+Tab 并跑独立消息循环（先退出常驻实例）");
    println!("  --icon-report        检查嵌入的图标资源与系统图标尺寸");
    println!("  --config-report      打印当前配置（文件路径与生效值）");
    println!();
    println!("配置文件：%APPDATA%\\whelper\\config.ini（设置页即时写回，手工改后重启生效）");
    println!("调试开关（环境变量）：TEXT_PICKER_DEBUG / TEXT_PICKER_THEME / SHOW_DESKTOP_DEBUG");
    println!("                      SWITCHER_DEBUG / WHELPER_HOOK_DEBUG");
}
