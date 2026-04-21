//! 加载系统中文字体（Windows 优先 msyh），避免 egui 默认字体显示中文为乱码/方块。

use std::fs;
use std::path::PathBuf;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

pub fn install_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let candidates: Vec<PathBuf> = cjk_font_candidates();
    let mut installed: Vec<&'static str> = Vec::new();

    for path in &candidates {
        if let Ok(bytes) = fs::read(path) {
            let key: &'static str = match path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "msyh.ttc" | "msyh.ttf" => "cjk_msyh",
                "simsun.ttc" | "simsun.ttf" => "cjk_simsun",
                "simhei.ttf" => "cjk_simhei",
                "deng.ttf" | "dengxian.ttf" => "cjk_deng",
                "pingfang.ttc" => "cjk_pingfang",
                "stheiti light.ttc" | "stheitisc-light.ttc" => "cjk_stheiti",
                "notosanscjk-regular.ttc" | "notosanscjksc-regular.otf" => "cjk_noto",
                "wqy-microhei.ttc" | "wqy-zenhei.ttc" => "cjk_wqy",
                _ => "cjk_fallback",
            };

            fonts
                .font_data
                .insert(key.to_string(), FontData::from_owned(bytes).into());

            installed.push(key);
            if installed.len() >= 2 {
                break;
            }
        }
    }

    if installed.is_empty() {
        return;
    }

    if let Some(prop) = fonts.families.get_mut(&FontFamily::Proportional) {
        for key in installed.iter().rev() {
            prop.insert(0, (*key).to_string());
        }
    }
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        for key in installed.iter().rev() {
            mono.push((*key).to_string());
        }
    }

    ctx.set_fonts(fonts);
}

fn cjk_font_candidates() -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = Vec::new();

    #[cfg(target_os = "windows")]
    {
        let win_fonts = std::env::var("WINDIR")
            .map(|d| PathBuf::from(d).join("Fonts"))
            .unwrap_or_else(|_| PathBuf::from(r"C:\Windows\Fonts"));
        for name in [
            "msyh.ttc",
            "msyh.ttf",
            "Deng.ttf",
            "Dengxian.ttf",
            "simhei.ttf",
            "simsun.ttc",
            "simsun.ttf",
        ] {
            list.push(win_fonts.join(name));
        }
    }

    #[cfg(target_os = "macos")]
    {
        for p in [
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/Library/Fonts/Songti.ttc",
        ] {
            list.push(PathBuf::from(p));
        }
    }

    #[cfg(target_os = "linux")]
    {
        for p in [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        ] {
            list.push(PathBuf::from(p));
        }
    }

    list
}
