use std::{
    io::{self, Read as _},
    time::Duration,
};

use eyre::{OptionExt, Result, WrapErr};
use hpx::Client;

use crate::{
    cli::{Cli, FormValue, OutputFormat},
    output::{
        TimingWaterfall, copy_to_clipboard, format_json_pretty, print_headers,
        print_redirect_history, print_request_line, print_status_line, write_body,
    },
};

pub(crate) async fn execute(cli: &Cli) -> Result<()> {
    let url = cli.url.as_deref().ok_or_eyre("no URL provided")?;
    let method_str = cli.method.as_str();
    let method = hpx_method(cli.method);

    let mut client_builder = Client::builder();

    // Proxy
    if let Some(ref proxy_url) = cli.proxy {
        let proxy = hpx::Proxy::all(proxy_url)
            .wrap_err_with(|| format!("invalid proxy URL: {proxy_url}"))?;
        client_builder = client_builder.proxy(proxy);
    }

    // Cookie store
    if !cli.cookie.is_empty() || cli.cookie_jar.is_some() {
        client_builder = client_builder.cookie_store(true);
    }

    // HTTP/3 version preference (`--http3` forces h3-only, `--prefer-http3`
    // enables Alt-Svc discovery with h2/h1 fallback; `--http3` wins if both
    // are passed).
    if cli.http3 {
        client_builder = client_builder.http3_only();
    } else if cli.prefer_http3 {
        client_builder = client_builder.prefer_http3();
    }

    // Browser emulation
    if let Some(ref emulation) = cli.emulation {
        let emulation = parse_emulation(emulation)
            .wrap_err_with(|| format!("invalid emulation profile: {emulation}"))?;
        client_builder = client_builder.emulation(emulation);
    }

    let client = client_builder.build()?;
    let mut builder = client.request(method, url.to_string());

    for (name, value) in cli.parsed_headers() {
        builder = builder.header(name, value);
    }

    // Manual cookies (add before request)
    for (name, value) in cli.parsed_cookies() {
        let cookie_str = format!("{name}={value}");
        builder = builder.header("Cookie", &cookie_str);
    }

    // Request body: --data or --json
    if let Some(ref data) = cli.data {
        let body = if data == "@-" {
            let mut buf = Vec::new();
            io::stdin()
                .read_to_end(&mut buf)
                .wrap_err("failed to read body from stdin")?;
            buf
        } else if let Some(stripped) = data.strip_prefix('@') {
            std::fs::read(stripped)
                .wrap_err_with(|| format!("failed to read body from file {stripped}"))?
        } else {
            data.as_bytes().to_vec()
        };
        builder = builder.body(body);
    }

    if let Some(ref json) = cli.json {
        let body = if json == "@-" {
            let mut buf = Vec::new();
            io::stdin()
                .read_to_end(&mut buf)
                .wrap_err("failed to read JSON body from stdin")?;
            buf
        } else if let Some(stripped) = json.strip_prefix('@') {
            std::fs::read(stripped)
                .wrap_err_with(|| format!("failed to read JSON body from file {stripped}"))?
        } else {
            json.as_bytes().to_vec()
        };
        builder = builder.header("Content-Type", "application/json");
        builder = builder.body(body);
    }

    // Form data
    if !cli.form.is_empty() {
        if cli.has_form_file_references() {
            // Use multipart form when any field has file references
            let mut form = hpx::multipart::Form::new();
            for (key, value) in cli.parsed_form_fields_with_files() {
                match value {
                    FormValue::Text(text) => {
                        form = form.text(key, text);
                    }
                    FormValue::File(path) => {
                        form = form.file(key, &path).await.wrap_err_with(|| {
                            format!("failed to read file {path} for form field")
                        })?;
                    }
                }
            }
            builder = builder.multipart(form);
        } else {
            // Plain urlencoded form data
            let fields: Vec<(String, String)> = cli.parsed_form_fields();
            builder = builder.form(&fields);
        }
    }

    // Multipart form data
    if !cli.multipart.is_empty() || !cli.multipart_file.is_empty() {
        let mut form = hpx::multipart::Form::new();
        for (key, value) in cli.parsed_multipart_fields() {
            form = form.text(key, value);
        }
        for (key, path) in cli.parsed_multipart_files() {
            form = form
                .file(key, &path)
                .await
                .wrap_err_with(|| format!("failed to add file {path} to multipart form"))?;
        }
        builder = builder.multipart(form);
    }

    if let Some(ref basic) = cli.basic {
        let (user, pass) = basic
            .split_once(':')
            .ok_or_eyre("invalid basic auth format, expected USER:PASS")?;
        builder = builder.basic_auth(user, Some(pass));
    }

    if let Some(ref bearer) = cli.bearer {
        builder = builder.bearer_auth(bearer);
    }

    if let Some(secs) = cli.timeout {
        builder = builder.timeout(Duration::from_secs_f64(secs));
    }

    if cli.dry_run {
        print_request_line(method_str, url, false);
        for (name, value) in cli.parsed_headers() {
            print_headers(&[(name, value)], true, false);
        }
        if !cli.form.is_empty() {
            eprintln!("[form data] {} fields", cli.form.len());
        }
        if !cli.multipart.is_empty() || !cli.multipart_file.is_empty() {
            let count = cli.multipart.len() + cli.multipart_file.len();
            eprintln!("[multipart] {count} fields");
        }
        if let Some(ref proxy) = cli.proxy {
            eprintln!("[proxy] {proxy}");
        }
        if cli.retry > 0 {
            eprintln!("[retry] {} attempts", cli.retry);
        }
        return Ok(());
    }

    let verbose = cli.verbose > 0;
    let color = crate::output::use_color(cli.color, crate::output::is_terminal());

    if verbose {
        print_request_line(method_str, url, color);
    }

    let mut waterfall = TimingWaterfall::new();

    let response = builder.send().await?;

    waterfall.mark_request_done();

    let status = response.status();
    let status_code = status.as_u16();
    let version = format!("{:?}", response.version());

    // Extract data before consuming response
    let response_headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("<binary>").to_string()))
        .collect();
    let set_cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok().map(String::from))
        .collect();
    let extensions = response.extensions().clone();

    // Display redirect history
    print_redirect_history(&extensions, verbose, color);

    if verbose || !cli.silent {
        print_status_line(status_code, &version, color);
        if verbose {
            print_headers(&response_headers, verbose, color);
        }
    }

    let bytes = response.bytes().await?;

    waterfall.mark_body_done();

    // Save cookies to jar if requested
    if let Some(ref jar_path) = cli.cookie_jar
        && !set_cookies.is_empty()
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(jar_path)?;
        for cookie_str in &set_cookies {
            writeln!(file, "{cookie_str}")?;
        }
        if verbose {
            eprintln!(
                "[cookie-jar] saved {} cookie(s) to {jar_path}",
                set_cookies.len()
            );
        }
    }

    let output_path = cli.output.as_deref();

    let body_bytes = match cli.format {
        OutputFormat::Json => {
            let pretty = format_json_pretty(&bytes)?;
            pretty.into_bytes()
        }
        OutputFormat::Auto => {
            if crate::output::looks_like_json_str(std::str::from_utf8(&bytes).unwrap_or("")) {
                match format_json_pretty(&bytes) {
                    Ok(pretty) => pretty.into_bytes(),
                    Err(_) => bytes.to_vec(),
                }
            } else {
                bytes.to_vec()
            }
        }
        OutputFormat::Text | OutputFormat::Raw => bytes.to_vec(),
    };

    if cli.clipboard {
        if let Ok(text) = std::str::from_utf8(&body_bytes) {
            copy_to_clipboard(text)?;
            if !cli.silent {
                eprintln!("Response copied to clipboard");
            }
        } else {
            eprintln!("Warning: response body is not valid UTF-8, cannot copy to clipboard");
        }
    }

    if cli.timing {
        waterfall.print(color);
    }

    write_body(&body_bytes, output_path)?;

    Ok(())
}

const fn hpx_method(method: crate::cli::Method) -> hpx::Method {
    match method {
        crate::cli::Method::Get => hpx::Method::GET,
        crate::cli::Method::Post => hpx::Method::POST,
        crate::cli::Method::Put => hpx::Method::PUT,
        crate::cli::Method::Delete => hpx::Method::DELETE,
        crate::cli::Method::Patch => hpx::Method::PATCH,
        crate::cli::Method::Head => hpx::Method::HEAD,
        crate::cli::Method::Options => hpx::Method::OPTIONS,
    }
}

/// All browser emulation profiles keyed by normalized name.
///
/// Keys are lowercase alphanumeric (no `_`, `-`, `.`, or spaces), so
/// `"chrome143"`, `"Chrome_143"`, and `"chrome-143"` all resolve. Keep this
/// table in sync with `hpx_emulation::Emulation` — every variant must appear
/// exactly once.
const EMULATION_PROFILES: &[(&str, hpx_emulation::Emulation)] = {
    use hpx_emulation::Emulation as E;
    &[
        ("chrome96", E::Chrome96),
        ("chrome100", E::Chrome100),
        ("chrome101", E::Chrome101),
        ("chrome104", E::Chrome104),
        ("chrome105", E::Chrome105),
        ("chrome106", E::Chrome106),
        ("chrome107", E::Chrome107),
        ("chrome108", E::Chrome108),
        ("chrome109", E::Chrome109),
        ("chrome110", E::Chrome110),
        ("chrome114", E::Chrome114),
        ("chrome116", E::Chrome116),
        ("chrome117", E::Chrome117),
        ("chrome118", E::Chrome118),
        ("chrome119", E::Chrome119),
        ("chrome120", E::Chrome120),
        ("chrome123", E::Chrome123),
        ("chrome124", E::Chrome124),
        ("chrome126", E::Chrome126),
        ("chrome127", E::Chrome127),
        ("chrome128", E::Chrome128),
        ("chrome129", E::Chrome129),
        ("chrome130", E::Chrome130),
        ("chrome131", E::Chrome131),
        ("chrome132", E::Chrome132),
        ("chrome133", E::Chrome133),
        ("chrome134", E::Chrome134),
        ("chrome135", E::Chrome135),
        ("chrome136", E::Chrome136),
        ("chrome137", E::Chrome137),
        ("chrome138", E::Chrome138),
        ("chrome139", E::Chrome139),
        ("chrome140", E::Chrome140),
        ("chrome141", E::Chrome141),
        ("chrome142", E::Chrome142),
        ("chrome143", E::Chrome143),
        ("chrome144", E::Chrome144),
        ("chrome145", E::Chrome145),
        ("chrome146", E::Chrome146),
        ("chrome147", E::Chrome147),
        ("chrome148", E::Chrome148),
        ("chrome149", E::Chrome149),
        ("edge96", E::Edge96),
        ("edge101", E::Edge101),
        ("edge122", E::Edge122),
        ("edge127", E::Edge127),
        ("edge131", E::Edge131),
        ("edge134", E::Edge134),
        ("edge135", E::Edge135),
        ("edge136", E::Edge136),
        ("edge137", E::Edge137),
        ("edge138", E::Edge138),
        ("edge139", E::Edge139),
        ("edge140", E::Edge140),
        ("edge141", E::Edge141),
        ("edge142", E::Edge142),
        ("edge143", E::Edge143),
        ("edge144", E::Edge144),
        ("edge145", E::Edge145),
        ("edge146", E::Edge146),
        ("edge147", E::Edge147),
        ("edge148", E::Edge148),
        ("opera116", E::Opera116),
        ("opera117", E::Opera117),
        ("opera118", E::Opera118),
        ("opera119", E::Opera119),
        ("opera120", E::Opera120),
        ("opera121", E::Opera121),
        ("opera122", E::Opera122),
        ("opera123", E::Opera123),
        ("opera124", E::Opera124),
        ("opera125", E::Opera125),
        ("opera126", E::Opera126),
        ("opera127", E::Opera127),
        ("opera128", E::Opera128),
        ("opera129", E::Opera129),
        ("opera130", E::Opera130),
        ("opera131", E::Opera131),
        ("safari14", E::Safari14),
        ("safari153", E::Safari15_3),
        ("safari155", E::Safari15_5),
        ("safari1561", E::Safari15_6_1),
        ("safari16", E::Safari16),
        ("safari165", E::Safari16_5),
        ("safari170", E::Safari17_0),
        ("safari1721", E::Safari17_2_1),
        ("safari1741", E::Safari17_4_1),
        ("safari175", E::Safari17_5),
        ("safari176", E::Safari17_6),
        ("safari18", E::Safari18),
        ("safari182", E::Safari18_2),
        ("safari183", E::Safari18_3),
        ("safari1831", E::Safari18_3_1),
        ("safari185", E::Safari18_5),
        ("safari26", E::Safari26),
        ("safari261", E::Safari26_1),
        ("safari262", E::Safari26_2),
        ("safari263", E::Safari26_3),
        ("safari264", E::Safari26_4),
        ("safari19", E::Safari19),
        ("safari20", E::Safari20),
        ("safari21", E::Safari21),
        ("safari22", E::Safari22),
        ("safari23", E::Safari23),
        ("safari24", E::Safari24),
        ("safari25", E::Safari25),
        ("safariios172", E::SafariIos17_2),
        ("safariios1741", E::SafariIos17_4_1),
        ("safariios165", E::SafariIos16_5),
        ("safariios1811", E::SafariIos18_1_1),
        ("safariipad18", E::SafariIPad18),
        ("safariipad26", E::SafariIPad26),
        ("safariipad262", E::SafariIPad26_2),
        ("safariipad263", E::SafariIPad26_3),
        ("safariipad264", E::SafariIPad26_4),
        ("safariios26", E::SafariIos26),
        ("safariios262", E::SafariIos26_2),
        ("safariios263", E::SafariIos26_3),
        ("safariios264", E::SafariIos26_4),
        ("safariios19", E::SafariIos19),
        ("safariipad19", E::SafariIPad19),
        ("safariios20", E::SafariIos20),
        ("safariipad20", E::SafariIPad20),
        ("safariios21", E::SafariIos21),
        ("safariipad21", E::SafariIPad21),
        ("safariios22", E::SafariIos22),
        ("safariipad22", E::SafariIPad22),
        ("safariios23", E::SafariIos23),
        ("safariipad23", E::SafariIPad23),
        ("safariios24", E::SafariIos24),
        ("safariipad24", E::SafariIPad24),
        ("safariios25", E::SafariIos25),
        ("safariipad25", E::SafariIPad25),
        ("firefox88", E::Firefox88),
        ("firefox109", E::Firefox109),
        ("firefox117", E::Firefox117),
        ("firefox128", E::Firefox128),
        ("firefox133", E::Firefox133),
        ("firefox135", E::Firefox135),
        ("firefoxprivate135", E::FirefoxPrivate135),
        ("firefoxandroid135", E::FirefoxAndroid135),
        ("firefox136", E::Firefox136),
        ("firefoxprivate136", E::FirefoxPrivate136),
        ("firefox137", E::Firefox137),
        ("firefox138", E::Firefox138),
        ("firefox139", E::Firefox139),
        ("firefox140", E::Firefox140),
        ("firefox141", E::Firefox141),
        ("firefox142", E::Firefox142),
        ("firefox143", E::Firefox143),
        ("firefox144", E::Firefox144),
        ("firefox145", E::Firefox145),
        ("firefox146", E::Firefox146),
        ("firefox147", E::Firefox147),
        ("firefox148", E::Firefox148),
        ("firefox149", E::Firefox149),
        ("firefox150", E::Firefox150),
        ("firefox151", E::Firefox151),
        ("okhttp39", E::OkHttp3_9),
        ("okhttp311", E::OkHttp3_11),
        ("okhttp313", E::OkHttp3_13),
        ("okhttp314", E::OkHttp3_14),
        ("okhttp49", E::OkHttp4_9),
        ("okhttp410", E::OkHttp4_10),
        ("okhttp412", E::OkHttp4_12),
        ("okhttp5", E::OkHttp5),
    ]
};

/// Parse a browser emulation profile string (e.g., "chrome143", "firefox88", "safari14").
///
/// Input is case-insensitive and ignores `_`, `-`, `.`, and spaces, so
/// `"Chrome_143"`, `"safari-ios-17.2"`, and `"okhttp_3.9"` all resolve.
fn parse_emulation(s: &str) -> Result<hpx_emulation::Emulation> {
    let normalized: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    EMULATION_PROFILES
        .iter()
        .find(|(name, _)| *name == normalized)
        .map(|(_, emulation)| *emulation)
        .ok_or_else(|| eyre::eyre!("unknown emulation profile: {s}"))
}

#[cfg(test)]
mod emulation_tests {
    use hpx_emulation::Emulation;

    use super::parse_emulation;

    #[test]
    fn parses_common_profiles() {
        assert!(matches!(
            parse_emulation("chrome143"),
            Ok(Emulation::Chrome143)
        ));
        assert!(matches!(
            parse_emulation("firefox88"),
            Ok(Emulation::Firefox88)
        ));
        assert!(matches!(
            parse_emulation("safari14"),
            Ok(Emulation::Safari14)
        ));
        assert!(matches!(parse_emulation("edge96"), Ok(Emulation::Edge96)));
        assert!(matches!(
            parse_emulation("opera119"),
            Ok(Emulation::Opera119)
        ));
        assert!(matches!(parse_emulation("okhttp5"), Ok(Emulation::OkHttp5)));
    }

    #[test]
    fn parsing_ignores_case_and_separators() {
        assert!(matches!(
            parse_emulation("Chrome_143"),
            Ok(Emulation::Chrome143)
        ));
        assert!(matches!(
            parse_emulation("safari-ios-17.2"),
            Ok(Emulation::SafariIos17_2)
        ));
        assert!(matches!(
            parse_emulation("okhttp_3.9"),
            Ok(Emulation::OkHttp3_9)
        ));
        assert!(matches!(
            parse_emulation("firefox_private_135"),
            Ok(Emulation::FirefoxPrivate135)
        ));
    }

    #[test]
    fn unknown_profile_is_rejected() {
        assert!(parse_emulation("netscape4").is_err());
        assert!(parse_emulation("").is_err());
    }
}
