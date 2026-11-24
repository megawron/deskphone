use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=po/LINGUAS");

    let linguas_path = Path::new("po/LINGUAS");
    let Ok(linguas) = std::fs::read_to_string(linguas_path) else {
        return;
    };

    for line in linguas.lines() {
        let lang = line.trim();
        if lang.is_empty() || lang.starts_with('#') {
            continue;
        }
        let po_file = format!("po/{}.po", lang);
        if !Path::new(&po_file).exists() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", po_file);

        let out_dir = format!("target/locale/{}/LC_MESSAGES", lang);
        if std::fs::create_dir_all(&out_dir).is_err() {
            continue;
        }
        let mo_file = format!("{}/deskphone.mo", out_dir);
        let _ = Command::new("msgfmt")
            .args(["-o", &mo_file, &po_file])
            .status();
    }
}
