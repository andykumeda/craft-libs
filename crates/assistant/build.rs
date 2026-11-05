//! Optional open fonts use the same build input as the other Craft apps.
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let dir = std::env::var_os("CRAFT_FONTS_DIR").filter(|value| !value.is_empty()).map(std::path::PathBuf::from);
    let required = std::env::var("CRAFT_FONTS_REQUIRED").is_ok_and(|value| value == "1");
    let result = helper_fonts(dir.as_deref(), required, craft_fonts_build::target_is_wasm())
        .and_then(|embed| embed.write("craft_fonts.rs").map_err(|error| error.to_string()));
    if let Err(error) = result {
        println!("cargo::error={error}");
    }
}

fn helper_fonts(dir: Option<&std::path::Path>, required: bool, wasm: bool) -> Result<craft_fonts_build::Embed, String> {
    let mut embed = craft_fonts_build::Embed::new().required(required).filter(|face| !is_excluded_family(&face.family));
    if let Some(dir) = dir {
        embed = embed.dir(dir);
        if !wasm {
            let chinese = dir.join("fonts/simplified-chinese-manifest.txt");
            match chinese.try_exists() {
                Ok(true) => embed = embed.additional_manifest("fonts/simplified-chinese-manifest.txt"),
                status => {
                    // Watch the existing directory so adding the optional manifest triggers a rebuild.
                    println!("cargo::rerun-if-changed={}", dir.join("fonts").display());
                    let reason = match status {
                        Err(error) => error.to_string(),
                        _ => "reviewed Simplified Chinese manifest is missing".into(),
                    };
                    let message = format!("{}: {reason}", chinese.display());
                    if required {
                        return Err(message);
                    }
                    println!("cargo::warning={message}; keeping permitted main fonts without a Simplified Chinese face");
                }
            }
        }
    }
    Ok(embed)
}

// The shared helper must satisfy every host's asset policy, including DeckCraft's exclusions.
fn is_excluded_family(family: &str) -> bool {
    matches!(family, "Source Sans 3" | "Source Serif 4")
        || family.starts_with("Source Han Sans")
        || family.starts_with("Source Han Serif")
        || family.starts_with("Noto Sans CJK")
        || family.starts_with("Noto Serif CJK")
        || family.starts_with("Noto Sans Mono CJK")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_helper_excludes_prohibited_families_and_retains_reviewed_alternatives() {
        for family in [
            "Source Sans 3",
            "Source Serif 4",
            "Source Han Sans SC",
            "Source Han Serif",
            "Noto Sans CJK SC",
            "Noto Serif CJK JP",
            "Noto Sans Mono CJK TC",
        ] {
            assert!(is_excluded_family(family), "{family}");
        }
        for family in ["BIZ UDPGothic", "BIZ UDMincho", "LXGW WenKai Lite", "Inter", "Noto Sans Arabic", "Noto Sans Thai"] {
            assert!(!is_excluded_family(family), "{family}");
        }
    }
    #[test]
    fn optional_old_input_keeps_japanese_and_required_input_needs_reviewed_hans() {
        let dir = std::env::temp_dir().join(format!("craft-helper-font-policy-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        std::fs::write(dir.join("fonts/synthetic.ttf"), b"synthetic-font-input").unwrap();
        std::fs::write(dir.join("fonts/manifest.txt"),"BIZ UDPGothic | Regular | fonts/synthetic.ttf | Jpan | OFL-1.1 | OFL.txt | 00 | src\nNoto Sans CJK SC | Regular | fonts/prohibited-missing.ttf | Hans | OFL-1.1 | OFL.txt | 00 | src\n").unwrap();
        let out = dir.join("table.rs");
        let optional = helper_fonts(Some(&dir), false, false).unwrap().write(&out).unwrap();
        assert_eq!(optional.iter().map(|face| face.family.as_str()).collect::<Vec<_>>(), ["BIZ UDPGothic"]);
        assert!(helper_fonts(Some(&dir), true, false).err().unwrap().contains("Simplified Chinese"));
        std::fs::write(
            dir.join("fonts/simplified-chinese-manifest.txt"),
            "LXGW WenKai Lite | Regular | fonts/synthetic.ttf | Hani,Hans | OFL-1.1 | OFL.txt | 00 | src\n",
        )
        .unwrap();
        let required = helper_fonts(Some(&dir), true, false).unwrap().write(&out).unwrap();
        assert_eq!(required.iter().map(|face| face.family.as_str()).collect::<Vec<_>>(), ["BIZ UDPGothic", "LXGW WenKai Lite"]);
        let wasm = helper_fonts(Some(&dir), true, true).unwrap().write(&out).unwrap();
        assert_eq!(wasm.iter().map(|face| face.family.as_str()).collect::<Vec<_>>(), ["BIZ UDPGothic"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
