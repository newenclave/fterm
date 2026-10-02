//! Puts the fterm icon into fterm.exe (a Windows resource).

fn main() {
    println!("cargo:rerun-if-changed=fterm.rc");
    println!("cargo:rerun-if-changed=../../assets/icon/fterm.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("fterm.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("cannot put the icon into fterm.exe");
    }
}
