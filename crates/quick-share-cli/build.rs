#![forbid(unsafe_code)]

fn main() {
    println!("cargo:rerun-if-changed=quick-share.rc");
    println!("cargo:rerun-if-changed=windows-manifest.xml");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_resource::compile("quick-share.rc", embed_resource::NONE)
            .manifest_required()
            .expect("compile Quick Share Windows resources");
    }
}
