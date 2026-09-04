fn main() {
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=assets/icons/windows/SerialX.ico");
        println!("cargo:rerun-if-changed=assets/icons/windows/serialx.rc");

        embed_resource::compile("assets/icons/windows/serialx.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed the Windows application icon");
    }
}
