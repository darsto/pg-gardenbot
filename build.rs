fn main() -> std::io::Result<()> {
    winresource::WindowsResource::new()
        .set_icon("icon.ico")
        .set_version_info(winresource::VersionInfo::PRODUCTVERSION, 0x0001000000000000)
        .compile()?;
    Ok(())
}
