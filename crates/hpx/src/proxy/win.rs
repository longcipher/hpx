pub(super) fn with_system(builder: &mut super::matcher::Builder) {
    let Ok(settings) = windows_registry::CURRENT_USER
        .open("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
    else {
        return;
    };

    if settings.get_u32("ProxyEnable").unwrap_or(0) == 0 {
        return;
    }

    if let Ok(val) = settings.get_string("ProxyServer") {
        super::sysproxy::apply_windows_proxy_server(builder, &val);
    }

    if builder.no.is_empty()
        && let Ok(val) = settings.get_string("ProxyOverride")
    {
        builder.no = super::sysproxy::parse_windows_proxy_override(&val);
    }
}
