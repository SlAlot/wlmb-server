// Network Panel fnOS adapter. LicenseRef-WLMB-NonCommercial.
fn main() {
    #[cfg(unix)]
    if let Err(error) = wlmb_server::fnos::run() {
        eprintln!("wlmb-fnos: {}", error.code);
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    {
        eprintln!("wlmb-fnos requires Linux");
        std::process::exit(2);
    }
}
