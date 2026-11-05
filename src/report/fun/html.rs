//! The offline HTML presentation and optional public-link metadata.

use anyhow::{Context, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::Url;

use super::*;

pub const SOCIAL_IMAGE_NAME: &str = "ai-stats-social-v1.png";
pub const SOCIAL_IMAGE: &[u8] = include_bytes!("../../../assets/ai-stats-social-v1.png");
const MASCOT: &str = include_str!("../../../assets/fun-mascot.svg");
const SHARE_TITLE: &str = "🤖 The logs have receipts. | AI Stats";
const IMAGE_ALT: &str = "A smiling robot printing receipts beside The logs have receipts. AI chat stats, swear jars, night owls and questionable life choices.";

const FONTS: &[(&str, &str, &[u8], &str)] = &[
    (
        "Bricolage Grotesque",
        "200 800",
        include_bytes!("../../../assets/fonts/bricolage-grotesque-latin.woff2"),
        include_str!("../../../assets/fonts/bricolage-grotesque-OFL.txt"),
    ),
    (
        "Space Grotesk",
        "300 700",
        include_bytes!("../../../assets/fonts/space-grotesk-latin.woff2"),
        include_str!("../../../assets/fonts/space-grotesk-OFL.txt"),
    ),
    (
        "DM Sans",
        "100 1000",
        include_bytes!("../../../assets/fonts/dm-sans-latin.woff2"),
        include_str!("../../../assets/fonts/dm-sans-OFL.txt"),
    ),
];

#[derive(Debug, Default)]
pub struct HtmlOptions {
    page_url: Option<Url>,
}

impl HtmlOptions {
    pub fn new(public_url: Option<&str>) -> Result<Self> {
        let page_url = public_url
            .map(|value| {
                let mut url =
                    Url::parse(value).context("--public-url must be an absolute HTTP(S) URL")?;
                if !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    bail!("--public-url must be an absolute HTTP(S) URL without credentials");
                }
                url.set_fragment(None);
                Ok(url)
            })
            .transpose()?;
        Ok(Self { page_url })
    }

    fn image_url(&self) -> Option<Url> {
        self.page_url.as_ref().map(|url| {
            url.join(SOCIAL_IMAGE_NAME)
                .expect("validated HTTP(S) URL is a valid base")
        })
    }
}

fn meta(out: &mut String, attribute: &str, name: &str, content: &str) {
    out.push_str(&format!(
        "<meta {attribute}=\"{name}\" content=\"{}\">\n",
        escape_html(content)
    ));
}

fn metadata(report: &FunReport, options: &HtmlOptions) -> String {
    let description = if report.total.counts.sessions == 0 {
        "A fresh AI chat recap. The swear jar is empty. For now.".to_string()
    } else {
        format!(
            "{}. {} tokens. ${:.2} in the swear jar. Night owls, AI whisperers & questionable life choices. Your AI chats, by the numbers.",
            count(report.total.counts.prompts, "prompt"),
            fmt_compact(report.total.tokens()),
            report.total.total(Cat::Swear) as f64 * SWEAR_JAR_USD,
        )
    };
    let mut out = String::new();
    for (attribute, name, value) in [
        ("name", "description", description.as_str()),
        ("name", "theme-color", "#11131d"),
        ("property", "og:type", "website"),
        ("property", "og:site_name", "AI Stats · Fun Stats"),
        ("property", "og:locale", "en_US"),
        ("property", "og:title", SHARE_TITLE),
        ("property", "og:description", description.as_str()),
        ("name", "twitter:title", SHARE_TITLE),
        ("name", "twitter:description", description.as_str()),
    ] {
        meta(&mut out, attribute, name, value);
    }
    if let (Some(page), Some(image)) = (&options.page_url, options.image_url()) {
        meta(&mut out, "property", "og:url", page.as_str());
        out.push_str(&format!(
            "<link rel=\"canonical\" href=\"{}\">\n",
            escape_html(page.as_str())
        ));
        meta(&mut out, "property", "og:image", image.as_str());
        if image.scheme() == "https" {
            meta(&mut out, "property", "og:image:secure_url", image.as_str());
        }
        for (name, value) in [
            ("og:image:type", "image/png"),
            ("og:image:width", "1200"),
            ("og:image:height", "630"),
            ("og:image:alt", IMAGE_ALT),
        ] {
            meta(&mut out, "property", name, value);
        }
        meta(&mut out, "name", "twitter:card", "summary_large_image");
        meta(&mut out, "name", "twitter:image", image.as_str());
        meta(&mut out, "name", "twitter:image:alt", IMAGE_ALT);
    } else {
        // Never invent a public host or advertise a data URL that crawlers
        // cannot fetch. A standalone file still has a useful text summary.
        meta(&mut out, "name", "twitter:card", "summary");
    }
    out
}

fn font_css() -> String {
    let mut css = String::new();
    for (family, weights, data, _) in FONTS {
        css.push_str(&format!(
            "@font-face {{ font-family: '{family}'; font-style: normal; font-weight: {weights}; font-display: swap; src: url(data:font/woff2;base64,{}) format('woff2'); }}\n",
            STANDARD.encode(data),
        ));
    }
    css
}

/// The font licenses travel with every self-contained file, as required by OFL.
fn font_credits() -> String {
    let mut out = String::from(
        "<details class=\"font-credits\"><summary>Type &amp; credits</summary><p>Bricolage Grotesque · Space Grotesk · DM Sans. Sourced from Google Fonts and embedded for offline viewing. SIL Open Font License 1.1.</p>\n",
    );
    for (family, _, _, license) in FONTS {
        out.push_str(&format!(
            "<details><summary>{family}</summary><pre>{}</pre></details>\n",
            escape_html(license)
        ));
    }
    out.push_str("</details>\n");
    out
}

fn stat_line(line: &str) -> String {
    // Escape each plain-text fragment; no formatting is read from log content.
    if let Some((label, text)) = line.split_once(" — ").or_else(|| line.split_once(": ")) {
        format!(
            "<li><strong class=\"stat-label\">{}</strong><span>{}</span></li>\n",
            escape_html(label),
            escape_html(text)
        )
    } else {
        format!("<li class=\"stat-prose\">{}</li>\n", escape_html(line))
    }
}

pub fn render_html(report: &FunReport, top: usize, options: &HtmlOptions) -> String {
    let mut out = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n\
         <meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; font-src data:; base-uri 'none'; form-action 'none'\">\n\
         <title>{}</title>\n{}<style>\n{}\n{}\n</style>\n</head>\n<body>\n\
         <input class=\"motion-toggle\" type=\"checkbox\" id=\"pause-motion\" aria-label=\"Pause decorative animations\">\n\
         <div class=\"ambient\" aria-hidden=\"true\"></div>\n<div class=\"page\">\n\
         <div class=\"masthead\"><a class=\"wordmark\" href=\"#top\">ai<span>stats</span><span class=\"wordmark-tag\"> / after hours</span></a>\n\
         <label class=\"motion-switch\" for=\"pause-motion\">Pause motion</label></div>\n\
         <header class=\"hero\" id=\"top\"><div class=\"hero-copy\">\n\
         <p class=\"eyebrow\"><span class=\"status-dot\" aria-hidden=\"true\"></span> AI Stats · Fun Stats</p>\n\
         <h1>The logs have <em>receipts.</em></h1>\n\
         <p class=\"deck\">Big prompts. Tiny patience.<br>A very serious report about your conversations with a robot.</p>\n",
        escape_html(TITLE),
        metadata(report, options),
        font_css(),
        include_str!("../fun.css"),
    );
    let has_sessions = report.total.counts.sessions > 0;
    if has_sessions {
        out.push_str(&format!(
            "<p class=\"period\">{}</p>\n",
            escape_html(&header(report, Style::Plain)[0])
        ));
    }
    out.push_str(&format!(
        "</div><div class=\"hero-art\"><div class=\"terminal-top\"><span aria-hidden=\"true\">● ● ●</span><span>vibe-check.log</span><span aria-hidden=\"true\">↗</span></div>{MASCOT}<p class=\"mascot-caption\"><span>&gt;_</span> emotionally available, statistically concerned</p></div></header>\n"
    ));
    let c = &report.total.counts;
    out.push_str("<div class=\"metrics\" aria-label=\"Report totals\">\n");
    for (icon, label, value, quip) in [
        ("💬", "Prompts sent", num(c.prompts), "a lot to unpack"),
        ("🗂️", "Sessions", num(c.sessions), "just one more thing"),
        (
            "🔧",
            "Tool calls",
            fmt_compact(c.tool_calls),
            "the robots were busy",
        ),
        (
            "🔥",
            "Tokens processed",
            fmt_compact(report.total.tokens()),
            "including cache hits",
        ),
    ] {
        out.push_str(&format!("<div class=\"metric\"><span class=\"metric-label\">{icon} {label}</span><strong>{value}</strong><span class=\"metric-quip\">{quip}</span></div>\n"));
    }
    out.push_str("</div>\n");
    // Keep the familiar book comparison and cache breakdown alongside the cards.
    for line in header(report, Style::Plain).into_iter().skip(2) {
        out.push_str(&format!(
            "<p class=\"token-note\">{}</p>\n",
            escape_html(&line)
        ));
    }

    let sections = sections(report, Style::Plain, top);
    if !sections.is_empty() {
        out.push_str("<nav aria-label=\"Report sections\">\n");
        for (index, section) in sections.iter().enumerate() {
            out.push_str(&format!(
                "<a href=\"#section-{index}\">{}</a>\n",
                escape_html(&section.title)
            ));
        }
        out.push_str("</nav>\n");
    }
    out.push_str("<main>\n");
    if !has_sessions {
        out.push_str(
            "<p class=\"empty\">No Claude Code or Codex sessions found in that period.</p>\n",
        );
    }
    for (index, section) in sections.iter().enumerate() {
        out.push_str(&format!(
            "<section class=\"card\" aria-labelledby=\"section-{index}\"><div class=\"section-heading\"><h2 id=\"section-{index}\">{}</h2><span class=\"chapter\">THE RECEIPTS / {:02}</span></div>\n",
            escape_html(&section.title), index + 1,
        ));
        if !section.chart.is_empty() {
            out.push_str(&format!(
                "<pre class=\"chart\" tabindex=\"0\" aria-label=\"Activity chart\">{}</pre>\n",
                escape_html(&section.chart.join("\n"))
            ));
        }
        if !section.lines.is_empty() {
            out.push_str("<ul class=\"stats\">\n");
            for line in &section.lines {
                out.push_str(&stat_line(line));
            }
            out.push_str("</ul>\n");
        }
        out.push_str("</section>\n");
    }
    out.push_str(&format!(
        "</main>\n<footer><p class=\"footer-motto\">Go on. Touch grass. The logs will be here.</p><p>{}</p><p>Generated {} · Activity charts use the host’s local time.</p>{}<a class=\"back-top\" href=\"#top\">Back to the receipts ↑</a></footer>\n</div>\n</body>\n</html>\n",
        escape_html(&footer(report)), report.generated_at.format("%Y-%m-%d %H:%M UTC"), font_credits(),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn social_urls_match_the_export_directory() {
        for (input, expected) in [
            (
                "https://example.com/",
                "https://example.com/ai-stats-social-v1.png",
            ),
            (
                "https://example.com/reports/",
                "https://example.com/reports/ai-stats-social-v1.png",
            ),
            (
                "https://example.com/reports/index.html?q=1#stats",
                "https://example.com/reports/ai-stats-social-v1.png",
            ),
        ] {
            let options = HtmlOptions::new(Some(input)).unwrap();
            assert_eq!(options.image_url().unwrap().as_str(), expected);
            assert!(options.page_url.unwrap().fragment().is_none());
        }
        for invalid in [
            "/stats.html",
            "data:image/png;base64,x",
            "javascript:alert(1)",
            "file:///tmp/stats.html",
            "https://user:secret@example.com/",
        ] {
            assert!(HtmlOptions::new(Some(invalid)).is_err());
        }
    }

    #[test]
    fn embedded_assets_have_the_expected_formats() {
        assert_eq!(&SOCIAL_IMAGE[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(SOCIAL_IMAGE[16..20].try_into().unwrap()),
            1200
        );
        assert_eq!(
            u32::from_be_bytes(SOCIAL_IMAGE[20..24].try_into().unwrap()),
            630
        );
        for (_, _, bytes, license) in FONTS {
            assert!(bytes.starts_with(b"wOF2"));
            assert!(license.contains("SIL OPEN FONT LICENSE Version 1.1"));
        }
    }
}
