fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "shell")]
    {
        println!("cargo:rerun-if-changed=../branding.json");
        let brand: serde_json::Value = serde_json::from_slice(&std::fs::read("../branding.json")?)?;
        let name = brand["displayName"]
            .as_str()
            .filter(|name| {
                !name.is_empty() && name.len() <= 120 && !name.chars().any(char::is_control)
            })
            .ok_or("Invalid branding displayName")?;
        let overrides = serde_json::json!({ "productName": name }).to_string();
        std::env::set_var("TAURI_CONFIG", &overrides);
        println!("cargo:rustc-env=TAURI_CONFIG={overrides}");
        println!("cargo:rustc-env=TONALLI_DISPLAY_NAME={name}");
        tauri_build::build();
    }
    Ok(())
}
