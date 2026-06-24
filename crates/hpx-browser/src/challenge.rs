//! Challenge detection and solving.

use async_trait::async_trait;

// ── Size gates ──────────────────────────────────────────────────────────

pub const INTERSTITIAL_MAX_BYTES: usize = 30 * 1024;
pub const BLOCKED_WORD_MAX_BYTES: usize = 5 * 1024;
pub const THIN_BODY_MAX_BYTES: usize = 1000;
pub const THIN_SHELL_MAX_BYTES: usize = 15 * 1024;
pub const SENSOR_SPLIT_BYTES: usize = 50 * 1024;

// ── Marker tables ───────────────────────────────────────────────────────

/// Any-size structural tokens — first match wins.
const UNAMBIGUOUS: &[(&str, &str)] = &[
    ("cf-browser-verification", "ManagedChallenge-CHL"),
    ("_cf_chl_opt", "ManagedChallenge-CHL"),
    ("/_sec/cp_challenge", "SecCpt-CHL"),
    ("ddcaptchaencoded", "Interstitial-CHL"),
];

/// AWS-WAF envelope markers — challenge only when co-signed by active loader.
const AWSWAF_MARKERS: &[&str] = &["gokuprops", "awswafcookiedomainlist"];

/// Active PoW loader substrings.
const AWSWAF_ACTIVE_LOADER: &[&str] =
    &["token.awswaf.com", "awswafintegration", "checkforcerefresh"];

/// Phrase markers — size-gated to < INTERSTITIAL_MAX_BYTES.
const PHRASE: &[(&str, &str)] = &[
    ("just a moment", "ManagedChallenge-CHL"),
    ("checking your browser", "ManagedChallenge-CHL"),
    ("captcha-delivery.com", "Interstitial-CHL"),
    ("press &amp; hold", "HoldChallenge-PaH"),
    ("pardon our interruption", "SensorChallenge-CHL"),
];

/// Small-body markers — size-gated, some require co-signals.
const SMALL_BODY: &[(&str, &str)] = &[
    ("akam/13", "SensorChallenge-CHL"),
    ("_abck", "SensorChallenge-CHL"),
    ("_kpsdk", "ScriptChallenge-CHL"),
    ("ips.js", "ScriptChallenge-CHL"),
    ("_pxhd", "BehaviorChallenge-CHL"),
    ("px-captcha", "BehaviorChallenge-CHL"),
    ("captcha", "captcha-CHL"),
    ("403 forbidden", "BLOCKED"),
    ("access denied", "BLOCKED"),
];

/// Co-signals required for `akam/13` to count as a challenge.
const SENSOR_CHALLENGE_COSIGNAL: &[&str] = &[
    "sensor_data",
    "bm-verify",
    "sec-if-cpt-container",
    "sec-cpt-if",
    "/_sec/cp_challenge",
    "pardon our interruption",
];

/// Co-signals that prove a genuinely interactive captcha (not invisible v3).
const INTERACTIVE_CAPTCHA_COSIGNAL: &[&str] = &[
    "api2/bframe",
    "api2/anchor",
    "hcaptcha.com",
    "cf-turnstile",
    "challenges.cloudflare.com/turnstile",
    "i'm not a robot",
    "i\u{2019}m not a robot",
    "verify you are human",
    "are you a robot",
    "select all images",
    "recaptcha challenge",
];

// ── Verdict ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeVerdict {
    Pass,
    RenderIncomplete,
    EdgeBlock,
    SensorFail,
    ThinShell,
    ChallengeIncomplete,
}

impl ChallengeVerdict {
    pub const fn is_challenge(self) -> bool {
        matches!(
            self,
            Self::EdgeBlock | Self::SensorFail | Self::ChallengeIncomplete
        )
    }
}

// ── Classification result ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineClass {
    pub tag: &'static str,
    pub verdict: ChallengeVerdict,
    pub len: usize,
}

// ── ChallengeKind / SolveOutcome / trait ────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengeKind {
    pub vendor: &'static str,
    pub sub_kind: &'static str,
}

impl ChallengeKind {
    pub const fn new(vendor: &'static str, sub_kind: &'static str) -> Self {
        Self { vendor, sub_kind }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolveOutcome {
    NotApplicable,
    InProgress,
    Solved,
    Unsolvable,
}

#[async_trait]
pub trait ChallengeSolver: Send + Sync {
    fn can_handle(&self, kind: &ChallengeKind) -> bool;
    async fn solve(&self, kind: &ChallengeKind, page: &mut crate::page::Page) -> SolveOutcome;
}

// ── Internal helpers ────────────────────────────────────────────────────

fn small_body_row_qualifies(needle: &str, lower: &str) -> bool {
    match needle {
        "akam/13" => SENSOR_CHALLENGE_COSIGNAL.iter().any(|c| lower.contains(c)),
        "captcha" => INTERACTIVE_CAPTCHA_COSIGNAL
            .iter()
            .any(|c| lower.contains(c)),
        _ => true,
    }
}

fn verdict_for(tag: &str, len: usize) -> ChallengeVerdict {
    match tag {
        "L3-RENDERED" if len < THIN_SHELL_MAX_BYTES => ChallengeVerdict::ThinShell,
        "L3-RENDERED" => ChallengeVerdict::Pass,
        "THIN-BODY" => ChallengeVerdict::RenderIncomplete,
        "ManagedChallenge-CHL" if len >= SENSOR_SPLIT_BYTES => {
            ChallengeVerdict::ChallengeIncomplete
        }
        _ if len < SENSOR_SPLIT_BYTES => ChallengeVerdict::EdgeBlock,
        _ => ChallengeVerdict::SensorFail,
    }
}

// ── Public API ──────────────────────────────────────────────────────────

pub fn engine_classify(body: &str) -> EngineClass {
    let lower = body.to_lowercase();
    let len = body.len();

    let tag: &'static str = 'tag: {
        for (n, t) in UNAMBIGUOUS {
            if lower.contains(n) {
                break 'tag t;
            }
        }
        if AWSWAF_MARKERS.iter().any(|n| lower.contains(n))
            && AWSWAF_ACTIVE_LOADER.iter().any(|n| lower.contains(n))
        {
            break 'tag "AWS-WAF-CHL";
        }
        if len < INTERSTITIAL_MAX_BYTES {
            for (n, t) in PHRASE {
                if lower.contains(n) {
                    break 'tag t;
                }
            }
            for (n, t) in SMALL_BODY {
                if lower.contains(n) && small_body_row_qualifies(n, &lower) {
                    break 'tag t;
                }
            }
        }
        if len < BLOCKED_WORD_MAX_BYTES && lower.contains("blocked") {
            break 'tag "BLOCKED";
        }
        if len < THIN_BODY_MAX_BYTES {
            break 'tag "THIN-BODY";
        }
        "L3-RENDERED"
    };

    EngineClass {
        tag,
        verdict: verdict_for(tag, len),
        len,
    }
}

pub fn is_managed_challenge_doc(body: &str) -> bool {
    body.contains("_cf_chl_opt")
        || body.contains("/cdn-cgi/challenge-platform/")
        || body.contains("cf-browser-verification")
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn page_is_challenge(body: &str) -> bool {
        engine_classify(body).verdict.is_challenge()
    }
    fn holistic_tag(body: &str) -> &'static str {
        engine_classify(body).tag
    }

    /// Helper: build a large rendered body embedding `seed`.
    fn big(seed: &str) -> String {
        let mut h = String::from("<html><body>");
        h.push_str(seed);
        for _ in 0..30000 {
            h.push_str("<div>actual rendered content paragraph</div>");
        }
        h.push_str("</body></html>");
        h
    }

    // ── 1. all_call_sites_agree ─────────────────────────────────────────

    #[test]
    fn all_call_sites_agree() {
        struct Case {
            name: &'static str,
            body: String,
            tag: &'static str,
            challenge: bool,
        }
        let cases = vec![
            Case {
                name: "empty",
                body: "<html></html>".into(),
                tag: "THIN-BODY",
                challenge: false,
            },
            Case {
                name: "cf small",
                body: "<html><body>Just a moment...</body></html>".into(),
                tag: "ManagedChallenge-CHL",
                challenge: true,
            },
            Case {
                name: "dd small",
                body: r#"<script src="https://geo.captcha-delivery.com/c"></script>"#.into(),
                tag: "Interstitial-CHL",
                challenge: true,
            },
            Case {
                name: "akam small",
                body: r#"<script src="/akam/13/abc"></script><form id="bm-verify"></form>"#.into(),
                tag: "SensorChallenge-CHL",
                challenge: true,
            },
            Case {
                name: "pxhd large benign",
                body: big(r#"<script>window._pxhd="sdk"</script>"#),
                tag: "L3-RENDERED",
                challenge: false,
            },
            Case {
                name: "just-a-moment large benign",
                body: big("<p>give us just a moment to load</p>"),
                tag: "L3-RENDERED",
                challenge: false,
            },
            Case {
                name: "grecaptcha config large",
                body: big(r#"<script>window.C={"googleRecaptcha":1}</script>"#),
                tag: "L3-RENDERED",
                challenge: false,
            },
        ];
        for c in cases {
            let ec = engine_classify(&c.body);
            assert_eq!(ec.tag, c.tag, "tag mismatch [{}]", c.name);
            assert_eq!(
                holistic_tag(&c.body),
                c.tag,
                "holistic disagrees [{}]",
                c.name
            );
            assert_eq!(
                page_is_challenge(&c.body),
                c.challenge,
                "page/holistic challenge-verdict disagree [{}] tag={}",
                c.name,
                ec.tag
            );
        }
    }

    // ── 2. FP-B2: literal strong markers size-gated ─────────────────────

    #[test]
    fn fp_b2_literal_strong_markers_size_gated() {
        let wf = big(
            r#"<script>window.__CONSENT={"_px3":"NECESSARY","px-captcha":"NECESSARY"};</script>"#,
        );
        assert_eq!(engine_classify(&wf).tag, "L3-RENDERED");
        assert_eq!(engine_classify(&wf).verdict, ChallengeVerdict::Pass);

        let dd = big(r#"<img src="https://x.captcha-delivery.com/pixel.gif">"#);
        assert_eq!(engine_classify(&dd).tag, "L3-RENDERED");
        assert_eq!(engine_classify(&dd).verdict, ChallengeVerdict::Pass);

        let px_chl = r#"<html><body><div id="px-captcha"></div><p>verifying</p></body></html>"#;
        assert_eq!(engine_classify(px_chl).tag, "BehaviorChallenge-CHL");
        assert!(engine_classify(px_chl).verdict.is_challenge());

        let pah = r#"<html><body><p>Press &amp; Hold to confirm</p></body></html>"#;
        assert_eq!(engine_classify(pah).tag, "HoldChallenge-PaH");
        assert!(engine_classify(pah).verdict.is_challenge());
    }

    // ── 3. FP-B5: AWS-WAF loader co-signed ──────────────────────────────

    #[test]
    fn fp_b5_awswaf_markers_loader_cosigned() {
        let mut solved = String::from(
            r#"<html><head><script>window.awsWafCookieDomainList=["redfin.com"];</script></head><body>
               <script>window.REDFIN_APP_NAME="customer-pages-personalization";</script>"#,
        );
        for _ in 0..6000 {
            solved.push_str("<div class='HomeCard'>real listing content here</div>");
        }
        solved.push_str("</body></html>");
        assert_eq!(engine_classify(&solved).tag, "L3-RENDERED");
        assert_eq!(engine_classify(&solved).verdict, ChallengeVerdict::Pass);

        let small_leftover = r#"<html><body><script>window.awsWafCookieDomainList=["redfin.com"];</script>
            <p>real content</p></body></html>"#;
        assert_ne!(engine_classify(small_leftover).tag, "AWS-WAF-CHL");
        assert!(!engine_classify(small_leftover).verdict.is_challenge());

        let stub = r#"<html><head><script src="https://x.token.awswaf.com/challenge.js"></script>
            <script>window.gokuProps={key:'a',context:'b',iv:'c'};</script></head>
            <body><div id="challenge-container"></div></body></html>"#;
        assert_eq!(engine_classify(stub).tag, "AWS-WAF-CHL");
        assert!(engine_classify(stub).verdict.is_challenge());

        let stub2 = r#"<html><body><script>var awsWafCookieDomainList=["a.com"];
            AwsWafIntegration.checkForceRefresh().then(()=>{});</script></body></html>"#;
        assert_eq!(engine_classify(stub2).tag, "AWS-WAF-CHL");
        assert!(engine_classify(stub2).verdict.is_challenge());
    }

    // ── 4. FP-B4: ManagedChallenge split from SensorFail ────────────────

    #[test]
    fn fp_b4_managed_incomplete_split_from_sensorfail() {
        let mut shell = String::from(
            r#"<html><head><title>Just a moment...</title></head><body>
               <script>window._cf_chl_opt={cvId:'3',cType:'managed'};</script>"#,
        );
        for _ in 0..2000 {
            shell.push_str("<div>cf challenge orchestrator shell padding</div>");
        }
        shell.push_str("</body></html>");
        assert!(shell.len() >= SENSOR_SPLIT_BYTES);
        let ec = engine_classify(&shell);
        assert_eq!(ec.tag, "ManagedChallenge-CHL");
        assert_eq!(ec.verdict, ChallengeVerdict::ChallengeIncomplete);
        assert_ne!(ec.verdict, ChallengeVerdict::SensorFail);
        assert_ne!(ec.verdict, ChallengeVerdict::Pass);
        assert!(ec.verdict.is_challenge());

        let stub =
            "<html><body><script>window._cf_chl_opt={}</script>Just a moment...</body></html>";
        assert_eq!(engine_classify(stub).verdict, ChallengeVerdict::EdgeBlock);

        let mut passed = String::from(
            r#"<html><body><script src="/cdn-cgi/challenge-platform/h/b/jsd"></script>"#,
        );
        for _ in 0..3000 {
            passed.push_str("<article>real rendered course catalog content</article>");
        }
        passed.push_str("</body></html>");
        assert!(passed.len() >= SENSOR_SPLIT_BYTES);
        assert_eq!(engine_classify(&passed).tag, "L3-RENDERED");
        assert_eq!(engine_classify(&passed).verdict, ChallengeVerdict::Pass);
    }

    // ── 5. FP-C2: managed challenge doc predicate ───────────────────────

    #[test]
    fn fp_c2_managed_challenge_doc_predicate() {
        assert!(is_managed_challenge_doc(
            r#"<script>window._cf_chl_opt={cvId:'3'};</script>"#
        ));
        assert!(is_managed_challenge_doc(
            r#"<script src="/cdn-cgi/challenge-platform/h/b/jsd/r/x"></script>"#
        ));
        assert!(is_managed_challenge_doc(
            r#"<html class="cf-browser-verification"><body>...</body></html>"#
        ));
        assert!(!is_managed_challenge_doc(
            "<html><body>fully rendered course catalog, no CF challenge</body></html>"
        ));
    }

    // ── 6. FP-B3: thin shell band ───────────────────────────────────────

    #[test]
    fn fp_b3_thin_shell_band() {
        let mut shell = String::from("<html><body>");
        for _ in 0..60 {
            shell.push_str("<div>spa hydration placeholder</div>");
        }
        shell.push_str("</body></html>");
        assert!(shell.len() > THIN_BODY_MAX_BYTES && shell.len() < THIN_SHELL_MAX_BYTES);
        let ec = engine_classify(&shell);
        assert_eq!(ec.tag, "L3-RENDERED");
        assert_eq!(ec.verdict, ChallengeVerdict::ThinShell);
        assert!(!ec.verdict.is_challenge());

        let mut full = String::from("<html><body>");
        for _ in 0..1000 {
            full.push_str("<article>real rendered content paragraph here</article>");
        }
        full.push_str("</body></html>");
        assert!(full.len() >= THIN_SHELL_MAX_BYTES);
        let fc = engine_classify(&full);
        assert_eq!(fc.tag, "L3-RENDERED");
        assert_eq!(fc.verdict, ChallengeVerdict::Pass);

        assert_eq!(
            engine_classify("<html></html>").verdict,
            ChallengeVerdict::RenderIncomplete
        );
    }

    // ── 7. FP-D2: unsolved managed challenge never passes ───────────────

    #[test]
    fn fp_d2_managed_unsolved_never_passes() {
        let stub =
            "<html><body>Just a moment...<script>window._cf_chl_opt={}</script></body></html>";
        let s = engine_classify(stub);
        assert!(s.verdict.is_challenge());
        assert_ne!(s.verdict, ChallengeVerdict::Pass);

        let mut shell = String::from(r#"<script>window._cf_chl_opt={cvId:'3'}</script>"#);
        for _ in 0..2500 {
            shell.push_str("<div>cf shell padding padding padding</div>");
        }
        assert!(shell.len() >= SENSOR_SPLIT_BYTES);
        let l = engine_classify(&shell);
        assert!(l.verdict.is_challenge());
        assert_ne!(l.verdict, ChallengeVerdict::Pass);
        assert_eq!(l.verdict, ChallengeVerdict::ChallengeIncomplete);
    }

    // ── 8. FP-Tier1: invisible recaptcha + akam/13 co-signal ────────────

    #[test]
    fn fp_t1_invisible_recaptcha_and_akam13_cosignal() {
        let mut spotify = String::from(
            r#"<html><head><style>.grecaptcha-badge { display: none !important }</style></head><body><textarea id="g-recaptcha-response-100000" name="g-recaptcha-response"></textarea><script src="https://www.gstatic.com/recaptcha/releases/abc/recaptcha__en.js"></script>"#,
        );
        for _ in 0..120 {
            spotify
                .push_str("<div class=\"sp-shell\">spotify web player hydration placeholder</div>");
        }
        spotify.push_str("</body></html>");
        assert!(spotify.len() > THIN_BODY_MAX_BYTES && spotify.len() < THIN_SHELL_MAX_BYTES);
        let s = engine_classify(&spotify);
        assert_eq!(s.tag, "L3-RENDERED");
        assert!(!s.verdict.is_challenge());
        assert_eq!(s.verdict, ChallengeVerdict::ThinShell);

        let real_cap = r#"<html><body><iframe src="https://www.google.com/recaptcha/api2/bframe?k=x"></iframe><p>Select all images with a bus — verify you are human</p></body></html>"#;
        assert_eq!(engine_classify(real_cap).tag, "captcha-CHL");
        assert!(engine_classify(real_cap).verdict.is_challenge());

        let mut bestbuy = String::from(
            r#"<html><head><script type="text/javascript" src="https://www.bestbuy.com/akam/13/62321f80" defer=""></script></head><body><h1>Choose a country</h1>"#,
        );
        for _ in 0..100 {
            bestbuy.push_str(
                "<a class=\"country\" href=\"/intl\">United States / Canada region selector</a>",
            );
        }
        bestbuy.push_str("</body></html>");
        assert!(bestbuy.len() > THIN_BODY_MAX_BYTES && bestbuy.len() < THIN_SHELL_MAX_BYTES);
        let b = engine_classify(&bestbuy);
        assert_eq!(b.tag, "L3-RENDERED");
        assert!(!b.verdict.is_challenge());

        let akam_chl = r#"<html><head><script src="/akam/13/abc"></script></head><body><form id="bm-verify"></form></body></html>"#;
        assert_eq!(engine_classify(akam_chl).tag, "SensorChallenge-CHL");
        assert!(engine_classify(akam_chl).verdict.is_challenge());
    }

    // ── 9. verdict_mapping_is_consistent ─────────────────────────────────

    #[test]
    fn verdict_mapping_is_consistent() {
        assert_eq!(
            engine_classify("<html></html>").verdict,
            ChallengeVerdict::RenderIncomplete
        );
        assert_eq!(
            engine_classify("<html><body>Just a moment...</body></html>").verdict,
            ChallengeVerdict::EdgeBlock
        );
        let mut big_dd = String::from(r#"<script>var ddcaptchaEncoded="z";</script>"#);
        for _ in 0..3000 {
            big_dd.push_str("<p>padding padding padding padding</p>");
        }
        assert!(big_dd.len() >= SENSOR_SPLIT_BYTES);
        assert_eq!(
            engine_classify(&big_dd).verdict,
            ChallengeVerdict::SensorFail
        );
    }

    // ── 10. AWS-WAF never passes unsolved ───────────────────────────────

    #[test]
    fn inverse_chl_awswaf_never_passes_unsolved() {
        let stub = r#"<html><head><script type="text/javascript">
            window.awsWafCookieDomainList = [];
            window.gokuProps = {"key":"AQ==","iv":"A6==","context":"gl=="};
            </script><script src="https://x.token.awswaf.com/x/challenge.js"></script></head>
            <body><script>AwsWafIntegration.checkForceRefresh().then(()=>{});</script></body></html>"#;
        let s = engine_classify(stub);
        assert_eq!(s.tag, "AWS-WAF-CHL");
        assert!(s.verdict.is_challenge());
        assert_ne!(s.verdict, ChallengeVerdict::Pass);

        let mut grown = String::from(
            r#"<script>window.gokuProps={"key":"AQ=="};window.awsWafCookieDomainList=[];
            AwsWafIntegration.checkForceRefresh();</script>
            <script src="https://x.token.awswaf.com/x/challenge.js"></script>"#,
        );
        for _ in 0..3000 {
            grown.push_str("<div>partially rendered challenge shell padding here</div>");
        }
        assert!(grown.len() >= SENSOR_SPLIT_BYTES);
        let g = engine_classify(&grown);
        assert_eq!(g.tag, "AWS-WAF-CHL");
        assert!(g.verdict.is_challenge());
        assert_ne!(g.verdict, ChallengeVerdict::Pass);

        let mut solved = String::from("<html><body>");
        for _ in 0..2000 {
            solved.push_str("<div class=\"product-card\">real amazon product listing</div>");
        }
        solved.push_str("</body></html>");
        assert!(solved.len() >= THIN_SHELL_MAX_BYTES);
        let v = engine_classify(&solved);
        assert_eq!(v.tag, "L3-RENDERED");
        assert_eq!(v.verdict, ChallengeVerdict::Pass);
    }

    // ── 11. tail-pin: known thin shells stay ThinShell ──────────────────

    #[test]
    fn tail_pin_known_thin_shells_stay_thinshell() {
        let mut duo = String::from(
            r#"<html><head><style>.grecaptcha-badge{display:none}</style></head><body>"#,
        );
        while duo.len() < 13_000 {
            duo.push_str("<div class=\"_2it2\">duolingo unsupported-browser shell</div>");
        }
        duo.push_str("</body></html>");
        assert!(
            duo.len() > THIN_BODY_MAX_BYTES && duo.len() < THIN_SHELL_MAX_BYTES,
            "duolingo shell must sit under the 15 KB ThinShell floor (len={})",
            duo.len()
        );
        let d = engine_classify(&duo);
        assert_eq!(d.tag, "L3-RENDERED");
        assert_eq!(d.verdict, ChallengeVerdict::ThinShell);
        assert!(!d.verdict.is_challenge());
    }

    // ── 12. DataDome captcha detection ──────────────────────────────────

    #[test]
    fn detect_datadome_captcha() {
        let body = r#"<script src="https://geo.captcha-delivery.com/captcha.js"></script><div id="ddcaptchaencoded">encoded_payload</div>"#;
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "Interstitial-CHL");
        assert!(ec.verdict.is_challenge());
    }

    // ── 13. SecCpt challenge ────────────────────────────────────────────

    #[test]
    fn detect_sec_cpt() {
        let body = r#"<html><body><div>loading...</div><script src="/_sec/cp_challenge/verify"></script></body></html>"#;
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "SecCpt-CHL");
        assert!(ec.verdict.is_challenge());
    }

    // ── 14. Kasada script challenge ─────────────────────────────────────

    #[test]
    fn detect_kasada_script() {
        let mut body = String::from(
            r#"<html><body><script>window._kpsdk={p:"abc"};</script><script src="/ips.js"></script>"#,
        );
        while body.len() < 2000 {
            body.push_str("<div>padding for threshold</div>");
        }
        body.push_str("</body></html>");
        let ec = engine_classify(&body);
        assert_eq!(ec.tag, "ScriptChallenge-CHL");
        assert!(ec.verdict.is_challenge());
    }

    // ── 15. PerimeterX challenge ────────────────────────────────────────

    #[test]
    fn detect_perimeterx_challenge() {
        let body = r#"<html><body><div id="px-captcha"></div><script>window._pxhd="abc";</script></body></html>"#;
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "BehaviorChallenge-CHL");
        assert!(ec.verdict.is_challenge());
    }

    // ── 16. Akamai sensor with co-signal ────────────────────────────────

    #[test]
    fn detect_akamai_sensor_with_cosignal() {
        let body = r#"<html><body><script src="/akam/13/pixel"></script><div id="sensor_data">payload</div></body></html>"#;
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "SensorChallenge-CHL");
        assert!(ec.verdict.is_challenge());
    }

    // ── 17. Akamai without co-signal is NOT a challenge ─────────────────

    #[test]
    fn akamai_without_cosignal_not_challenge() {
        let body =
            r#"<html><body><script src="/akam/13/pixel"></script><p>Welcome</p></body></html>"#;
        let ec = engine_classify(body);
        assert_ne!(ec.tag, "SensorChallenge-CHL");
    }

    // ── 18. Blocked word detection ──────────────────────────────────────

    #[test]
    fn detect_blocked_small_body() {
        let body = "<html><body><h1>403 Forbidden</h1><p>Access Denied</p></body></html>";
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "BLOCKED");
        assert!(ec.verdict.is_challenge());
    }

    // ── 19. Blocked word size-gated ─────────────────────────────────────

    #[test]
    fn blocked_word_size_gated() {
        let body = big("Access Denied");
        let ec = engine_classify(&body);
        assert_eq!(ec.tag, "L3-RENDERED");
        assert_eq!(ec.verdict, ChallengeVerdict::Pass);
    }

    // ── 20. cf-browser-verification ─────────────────────────────────────

    #[test]
    fn detect_cf_browser_verification() {
        let body =
            r#"<html class="cf-browser-verification"><body>Checking your browser...</body></html>"#;
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "ManagedChallenge-CHL");
        assert!(ec.verdict.is_challenge());
    }

    // ── 21. checking your browser phrase ────────────────────────────────

    #[test]
    fn detect_checking_your_browser() {
        let body =
            "<html><body><p>Checking your browser before accessing the site...</p></body></html>";
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "ManagedChallenge-CHL");
        assert!(ec.verdict.is_challenge());
    }

    // ── 22. hcaptcha detection ──────────────────────────────────────────

    #[test]
    fn detect_hcaptcha() {
        let body = r#"<html><body><iframe src="https://hcaptcha.com/captcha/v1/challenge"></iframe></body></html>"#;
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "captcha-CHL");
        assert!(ec.verdict.is_challenge());
    }

    // ── 23. cf-turnstile detection ──────────────────────────────────────

    #[test]
    fn detect_cf_turnstile() {
        let mut body = String::from(
            r#"<html><body><div class="cf-turnstile" data-sitekey="x"></div><p>captcha verification</p>"#,
        );
        while body.len() < 2000 {
            body.push_str("<p>Verify you are human to continue browsing this site</p>");
        }
        body.push_str("</body></html>");
        let ec = engine_classify(&body);
        assert_eq!(ec.tag, "captcha-CHL");
        assert!(ec.verdict.is_challenge());
    }

    // ── 24. pardon our interruption ─────────────────────────────────────

    #[test]
    fn detect_pardon_interruption() {
        let body = "<html><body><p>Pardon our interruption, verifying access</p></body></html>";
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "SensorChallenge-CHL");
        assert!(ec.verdict.is_challenge());
    }

    // ── 25. normal HTML is L3-RENDERED ──────────────────────────────────

    #[test]
    fn normal_html_passes() {
        let mut body = String::from("<html><body>");
        for _ in 0..400 {
            body.push_str("<p>Normal rendered content paragraph with enough text to fill.</p>");
        }
        body.push_str("</body></html>");
        assert!(body.len() >= THIN_SHELL_MAX_BYTES, "body must be >= 15KB");
        let ec = engine_classify(&body);
        assert_eq!(ec.tag, "L3-RENDERED");
        assert_eq!(ec.verdict, ChallengeVerdict::Pass);
    }

    // ── 26. BDD: CF managed challenge ───────────────────────────────────

    #[test]
    fn bdd_cf_managed_challenge() {
        let body = r#"<html><body><script>window._cf_chl_opt={cvId:'3'};</script>
            <script src="/cdn-cgi/challenge-platform/h/b/jsd"></script></body></html>"#;
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "ManagedChallenge-CHL");
        assert_eq!(ec.verdict, ChallengeVerdict::EdgeBlock);
    }

    // ── 27. BDD: AWS-WAF challenge ──────────────────────────────────────

    #[test]
    fn bdd_aws_waf_challenge() {
        let body = r#"<html><body>
            <script>window.gokuProps={key:'a'};window.awsWafCookieDomainList=["x.com"];</script>
            <script src="https://x.token.awswaf.com/challenge.js"></script>
            <script>AwsWafIntegration.checkForceRefresh();</script>
        </body></html>"#;
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "AWS-WAF-CHL");
        assert_eq!(ec.verdict, ChallengeVerdict::EdgeBlock);
    }

    // ── 28. BDD: clean response ─────────────────────────────────────────

    #[test]
    fn bdd_clean_response() {
        let mut body = String::from("<html><body>");
        for _ in 0..400 {
            body.push_str("<p>Normal content with enough text to exceed the 15KB threshold.</p>");
        }
        body.push_str("</body></html>");
        assert!(body.len() >= THIN_SHELL_MAX_BYTES);
        let ec = engine_classify(&body);
        assert_eq!(ec.tag, "L3-RENDERED");
        assert_eq!(ec.verdict, ChallengeVerdict::Pass);
    }

    // ── 29. BDD: thin body ──────────────────────────────────────────────

    #[test]
    fn bdd_thin_body() {
        let body = "<html><body>tiny</body></html>";
        let ec = engine_classify(body);
        assert_eq!(ec.tag, "THIN-BODY");
        assert_eq!(ec.verdict, ChallengeVerdict::RenderIncomplete);
    }

    // ── 30. ChallengeVerdict::is_challenge coverage ─────────────────────

    #[test]
    fn is_challenge_coverage() {
        assert!(!ChallengeVerdict::Pass.is_challenge());
        assert!(!ChallengeVerdict::RenderIncomplete.is_challenge());
        assert!(!ChallengeVerdict::ThinShell.is_challenge());
        assert!(ChallengeVerdict::EdgeBlock.is_challenge());
        assert!(ChallengeVerdict::SensorFail.is_challenge());
        assert!(ChallengeVerdict::ChallengeIncomplete.is_challenge());
    }

    // ── 31. EngineClamp len field ───────────────────────────────────────

    #[test]
    fn engine_class_len_matches_body() {
        let body = "hello";
        let ec = engine_classify(body);
        assert_eq!(ec.len, 5);
    }

    // ── proptest: engine_classify never panics on arbitrary input ────────

    #[cfg(feature = "proptest")]
    mod proptests {
        use proptest::prelude::*;

        use super::*;

        proptest! {
            #[test]
            fn engine_classify_never_panics(body in ".*") {
                let ec = engine_classify(&body);
                // Must always return a valid tag
                let _ = ec.tag;
                let _ = ec.verdict;
                let _ = ec.len;
            }
        }
    }
}
