use mediahub_core::{S3CorsConfiguration, S3CorsMethod, S3CorsRule};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct S3CorsDecision {
    pub(crate) allow_origin: String,
    pub(crate) allow_methods: Vec<S3CorsMethod>,
    pub(crate) allow_headers: Vec<String>,
    pub(crate) expose_headers: Vec<String>,
    pub(crate) max_age_seconds: Option<u32>,
}

#[must_use]
pub(crate) fn evaluate_s3_cors_actual_request(
    configuration: &S3CorsConfiguration,
    origin: &str,
    method: S3CorsMethod,
) -> Option<S3CorsDecision> {
    find_matching_rule(configuration, origin, method, &[])
        .map(|rule| decision_for(rule, origin, rule.allowed_methods().to_vec(), Vec::new()))
}

#[must_use]
pub(crate) fn evaluate_s3_cors_preflight(
    configuration: &S3CorsConfiguration,
    origin: &str,
    requested_method: S3CorsMethod,
    requested_headers: &[&str],
) -> Option<S3CorsDecision> {
    find_matching_rule(configuration, origin, requested_method, requested_headers).map(|rule| {
        decision_for(
            rule,
            origin,
            vec![requested_method],
            requested_headers
                .iter()
                .map(|header| (*header).to_owned())
                .collect(),
        )
    })
}

fn find_matching_rule<'a>(
    configuration: &'a S3CorsConfiguration,
    origin: &str,
    method: S3CorsMethod,
    requested_headers: &[&str],
) -> Option<&'a S3CorsRule> {
    configuration.rules().iter().find(|rule| {
        rule.allowed_origins()
            .iter()
            .any(|pattern| wildcard_matches(pattern, origin))
            && rule.allowed_methods().contains(&method)
            && requested_headers.iter().all(|requested_header| {
                !requested_header.is_empty()
                    && requested_header.is_ascii()
                    && rule.allowed_headers().iter().any(|pattern| {
                        wildcard_matches_ascii_case_insensitive(pattern, requested_header)
                    })
            })
    })
}

fn decision_for(
    rule: &S3CorsRule,
    origin: &str,
    allow_methods: Vec<S3CorsMethod>,
    allow_headers: Vec<String>,
) -> S3CorsDecision {
    S3CorsDecision {
        // S3 echoes the request Origin after a configured wildcard has matched.
        allow_origin: origin.to_owned(),
        allow_methods,
        allow_headers,
        expose_headers: rule.expose_headers().to_vec(),
        max_age_seconds: rule.max_age_seconds(),
    }
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return pattern == value;
    };

    value.len() >= prefix.len() + suffix.len()
        && value.starts_with(prefix)
        && value.ends_with(suffix)
}

fn wildcard_matches_ascii_case_insensitive(pattern: &str, value: &str) -> bool {
    wildcard_matches(&pattern.to_ascii_lowercase(), &value.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(
        id: &str,
        methods: &[S3CorsMethod],
        origins: &[&str],
        allowed_headers: &[&str],
        expose_headers: &[&str],
        max_age_seconds: Option<u32>,
    ) -> S3CorsRule {
        S3CorsRule::new(
            Some(id.to_owned()),
            methods.to_vec(),
            origins.iter().map(|value| (*value).to_owned()).collect(),
            allowed_headers
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            expose_headers
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            max_age_seconds,
        )
        .expect("valid CORS rule")
    }

    fn configuration(rules: Vec<S3CorsRule>) -> S3CorsConfiguration {
        S3CorsConfiguration::new(rules).expect("valid CORS configuration")
    }

    #[test]
    fn actual_request_matching_is_table_driven() {
        let configuration = configuration(vec![rule(
            "actual",
            &[S3CorsMethod::Get, S3CorsMethod::Head],
            &["https://*.example.com", "https://exact.example.net"],
            &["x-amz-*"],
            &["etag", "x-amz-version-id"],
            Some(600),
        )]);
        let cases = [
            (
                "exact origin",
                "https://exact.example.net",
                S3CorsMethod::Get,
                true,
            ),
            (
                "single-star origin",
                "https://images.example.com",
                S3CorsMethod::Head,
                true,
            ),
            (
                "origin comparison is case-sensitive",
                "HTTPS://images.example.com",
                S3CorsMethod::Get,
                false,
            ),
            (
                "origin suffix must match",
                "https://images.example.org",
                S3CorsMethod::Get,
                false,
            ),
            (
                "method must match",
                "https://images.example.com",
                S3CorsMethod::Put,
                false,
            ),
        ];

        for (name, origin, method, should_match) in cases {
            let decision = evaluate_s3_cors_actual_request(&configuration, origin, method);
            assert_eq!(decision.is_some(), should_match, "{name}");
            if let Some(decision) = decision {
                assert_eq!(decision.allow_origin, origin, "{name}");
                assert_eq!(
                    decision.allow_methods,
                    [S3CorsMethod::Get, S3CorsMethod::Head],
                    "{name}"
                );
                assert!(decision.allow_headers.is_empty(), "{name}");
                assert_eq!(
                    decision.expose_headers,
                    ["etag", "x-amz-version-id"],
                    "{name}"
                );
                assert_eq!(decision.max_age_seconds, Some(600), "{name}");
            }
        }
    }

    #[test]
    fn preflight_matching_is_table_driven() {
        let configuration = configuration(vec![rule(
            "preflight",
            &[S3CorsMethod::Get, S3CorsMethod::Put],
            &["https://upload.example.com"],
            &["content-type", "x-*-meta", "x-amz-*"],
            &["etag"],
            Some(3_600),
        )]);
        let cases: [(&str, &[&str], bool); 6] = [
            ("no requested headers", &[], true),
            ("header names ignore ASCII case", &["Content-Type"], true),
            ("wildcard matches a middle segment", &["X-Image-Meta"], true),
            ("wildcard may match zero bytes", &["x--meta"], true),
            (
                "every requested header is allowed",
                &["content-type", "X-Amz-Checksum-Sha256"],
                true,
            ),
            (
                "one denied header rejects the rule",
                &["content-type", "authorization"],
                false,
            ),
        ];

        for (name, requested_headers, should_match) in cases {
            let decision = evaluate_s3_cors_preflight(
                &configuration,
                "https://upload.example.com",
                S3CorsMethod::Put,
                requested_headers,
            );
            assert_eq!(decision.is_some(), should_match, "{name}");
            if let Some(decision) = decision {
                assert_eq!(
                    decision.allow_origin, "https://upload.example.com",
                    "{name}"
                );
                assert_eq!(decision.allow_methods, [S3CorsMethod::Put], "{name}");
                assert_eq!(decision.allow_headers, requested_headers, "{name}");
                assert_eq!(decision.expose_headers, ["etag"], "{name}");
                assert_eq!(decision.max_age_seconds, Some(3_600), "{name}");
            }
        }
    }

    #[test]
    fn the_first_matching_rule_wins_for_actual_and_preflight_requests() {
        let configuration = configuration(vec![
            rule(
                "first",
                &[S3CorsMethod::Get],
                &["*"],
                &["x-amz-*"],
                &["x-first"],
                Some(10),
            ),
            rule(
                "second",
                &[S3CorsMethod::Get],
                &["https://app.example.com"],
                &["*"],
                &["x-second"],
                Some(20),
            ),
        ]);

        let actual = evaluate_s3_cors_actual_request(
            &configuration,
            "https://app.example.com",
            S3CorsMethod::Get,
        )
        .expect("actual request matches");
        let preflight = evaluate_s3_cors_preflight(
            &configuration,
            "https://app.example.com",
            S3CorsMethod::Get,
            &["X-Amz-Date"],
        )
        .expect("preflight request matches");

        for decision in [actual, preflight] {
            assert_eq!(decision.allow_origin, "https://app.example.com");
            assert_eq!(decision.expose_headers, ["x-first"]);
            assert_eq!(decision.max_age_seconds, Some(10));
        }
    }

    #[test]
    fn a_preflight_skips_an_earlier_rule_when_its_headers_do_not_match() {
        let configuration = configuration(vec![
            rule(
                "first",
                &[S3CorsMethod::Put],
                &["*"],
                &["content-type"],
                &["x-first"],
                None,
            ),
            rule(
                "second",
                &[S3CorsMethod::Put],
                &["*"],
                &["x-amz-*"],
                &["x-second"],
                Some(30),
            ),
        ]);

        let decision = evaluate_s3_cors_preflight(
            &configuration,
            "https://app.example.com",
            S3CorsMethod::Put,
            &["X-Amz-Date"],
        )
        .expect("second rule matches");

        assert_eq!(decision.expose_headers, ["x-second"]);
        assert_eq!(decision.max_age_seconds, Some(30));
    }
}
