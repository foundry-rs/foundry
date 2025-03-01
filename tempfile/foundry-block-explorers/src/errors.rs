use alloy_chains::Chain;
use alloy_primitives::Address;
use std::env::VarError;

#[derive(Debug, thiserror::Error)]
pub enum EtherscanError {
    #[error("Chain {0} not supported")]
    ChainNotSupported(Chain),
    #[error("Contract execution call failed: {0}")]
    ExecutionFailed(String),
    #[error("Balance failed")]
    BalanceFailed,
    #[error("Block by timestamp failed")]
    BlockNumberByTimestampFailed,
    #[error("Transaction receipt failed")]
    TransactionReceiptFailed,
    #[error("Gas estimation failed")]
    GasEstimationFailed,
    #[error("Bad status code: {0}")]
    BadStatusCode(String),
    #[error(transparent)]
    EnvVarNotFound(#[from] VarError),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error("Failed to deserialize content: {error}\n{content}")]
    Serde { error: serde_json::Error, content: String },
    #[error("Contract source code not verified: {0}")]
    ContractCodeNotVerified(Address),
    #[error("Response result is unexpectedly empty: status={status}, message={message}")]
    EmptyResult { status: String, message: String },
    #[error("Rate limit exceeded")]
    RateLimitExceeded,
    #[error(transparent)]
    IO(#[from] std::io::Error),
    #[error("Local networks (e.g. anvil, ganache, geth --dev) cannot be indexed by etherscan")]
    LocalNetworksNotSupported,
    #[error("Received error response: status={status},message={message}, result={result:?}")]
    ErrorResponse { status: String, message: String, result: Option<String> },
    #[error("Unknown error: {0}")]
    Unknown(String),
    #[error("Missing field: {0}")]
    Builder(String),
    #[error("Missing solc version: {0}")]
    MissingSolcVersion(String),
    #[error("Invalid API Key")]
    InvalidApiKey,
    #[error("Sorry, you have been blocked by Cloudflare, See also https://community.cloudflare.com/t/sorry-you-have-been-blocked/110790")]
    BlockedByCloudflare,
    #[error("The Requested prompted a cloudflare captcha security challenge to review the security of your connection before proceeding.")]
    CloudFlareSecurityChallenge,
    #[error("Received `Page not found` response. API server is likely down")]
    PageNotFound,
    #[error("Contract was not found: {0}")]
    ContractNotFound(Address),
}

/// etherscan/polyscan is protected by cloudflare, which can lead to html responses like `Sorry, you have been blocked` See also <https://community.cloudflare.com/t/sorry-you-have-been-blocked/110790>
///
/// This returns true if the `txt` is a cloudflare error response
pub(crate) fn is_blocked_by_cloudflare_response(txt: &str) -> bool {
    txt.to_lowercase().contains("sorry, you have been blocked")
}

/// etherscan/polyscan is protected by cloudflare, which can require captchas to "review the
/// security of your connection before proceeding"
pub(crate) fn is_cloudflare_security_challenge(txt: &str) -> bool {
    txt.contains("https://www.cloudflare.com?utm_source=challenge")
        || txt.to_lowercase().contains("checking if the site connection is secure")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_cloudflare_security_challenge() {
        let res = "<!DOCTYPE html><html lang=\"en-US\"><head>    <title>Just a moment...</title>    <meta http-equiv=\"Content-Type\" content=\"text/html; charset=UTF-8\">    <meta http-equiv=\"X-UA-Compatible\" content=\"IE=Edge\">    <meta name=\"robots\" content=\"noindex,nofollow\">    <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">    <link href=\"/cdn-cgi/styles/challenges.css\" rel=\"stylesheet\">    </head><body class=\"no-js\">    <div class=\"main-wrapper\" role=\"main\">    <div class=\"main-content\">        <h1 class=\"zone-name-title h1\">            <img class=\"heading-favicon\" src=\"/favicon.ico\" alt=\"Icon for api-goerli.etherscan.io\"                 onerror=\"this.onerror=null;this.parentNode.removeChild(this)\">            api-goerli.etherscan.io        </h1>        <h2 class=\"h2\" id=\"challenge-running\">            Checking if the site connection is secure        </h2>        <noscript>            <div id=\"challenge-error-title\">                <div class=\"h2\">                    <span class=\"icon-wrapper\">                        <div class=\"heading-icon warning-icon\"></div>                    </span>                    <span id=\"challenge-error-text\">                        Enable JavaScript and cookies to continue                    </span>                </div>            </div>        </noscript>        <div id=\"trk_jschal_js\" style=\"display:none;background-image:url('/cdn-cgi/images/trace/captcha/nojs/transparent.gif?ray=794294b0ff122cc8')\"></div>        <div id=\"challenge-body-text\" class=\"core-msg spacer\">            api-goerli.etherscan.io needs to review the security of your connection before proceeding.        </div>        <form id=\"challenge-form\" action=\"/api/?__cf_chl_f_tk=3e8YnNWEoJpt7yhj9ZB_z7nPP6BpgWkrEO9fxS_FHfs-1675505052-0-gaNycGzNCv0\" method=\"POST\" enctype=\"application/x-www-form-urlencoded\">            <input type=\"hidden\" name=\"md\" value=\"kphchgOl8CtVKUgr8zjngomaIt8zm7QzUkLvDaiTi.Q-1675505052-0-Ae-rscWjz835ha6epVnom-tK6T9VtNzQrwuMr1t3Ajth5BX5PQNhXdiKh7SzcaqcQ1sNxb_BVXo_zQsvD9DKNXvBQXaSAWho5s2SvMaYxzNolLg01kyTNP0b9YiAKirGukD524hsIbyLgZRR3H8VDEfVwpDADGKb3MhA5rwiHE8oGZcLgjcVjj4nXGYWGwKOjo6LyPWuoRLSHrMRYzsrizm-GQ1fafuos6afDCPTV6B05TLLodI4z2wNbeirPKaZGL3rScvPR_F-CGS1vWFJvLxlqJ0dulyMP_gehm_1AJm-FPevnqJoCgyU51wBgCkkVfOmwj31xAFhk1SaWUIrjxhN-PRH2RUMtnEdWHRffX20JCcc4OJ2HbTZ-J7u9U713PG-A3SPE2Az_nUoVOeJ90_aytPlYLppspJCYFceac4VUUldRVlGyHGkZBafp9xA0wOJzAhDy8K9PiVfRkso-dyM1t5tNr4qCtg3HutQ5u2E\">            <input type=\"hidden\" name=\"r\" value=\"qppUKLcDpkximVAwuJGymQi_4Iem1Vkg1AKAEuUoy4o-1675505052-0-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\">        </form>    </div></div><script>    (function(){        window._cf_chl_opt={            cvId: '2',            cZone: 'api-goerli.etherscan.io',            cType: 'interactive',            cNounce: '15579',            cRay: '794294b0ff122cc8',            cHash: '8512e1389fe72aa',            cUPMDTk: \"\\/api\\/?__cf_chl_tk=3e8YnNWEoJpt7yhj9ZB_z7nPP6BpgWkrEO9fxS_FHfs-1675505052-0-gaNycGzNCv0\",            cFPWv: 'b',            cTTimeMs: '1000',            cMTimeMs: '0',            cTplV: 4,            cTplB: 'cf',            cRq: {                ru: 'aHR0cHM6Ly9hcGktZ29lcmxpLmV0aGVyc2Nhbi5pby9hcGkv',                ra: 'Tk9fVUE=',                rm: 'UE9TVA==',                d: 'iVIFhJNH894P6YjM/iKMo5udA3+52aRoalezmALjhknKSbsB4BNUHq4xo08NpSCwVCpWCaVZhDDFDyjuk/b8Qc7g0gq6r2Nvql1jbmMxMYtueXQZm3u2y81oNhVmL0b0KzAx055+4tUbIe8wMboAsOqvpV8HaPpnIEcjO41rPHALM1ut6PKgqUidMU2DTeQbXhyZHbsrIqMsPbNpBQ4G9nepnyEEiOKZVICjhh2ic0LlLh9dcVTFH7r/ROe+A0rslaTcS+rrPg02VaiykvjlAHsbI+LNk6L/VBwZiX2BmQzErGaj3BF04hqKLou63qO55g6mwwiNfV/clJ8CivVcpJiZqEVpSpINLP3WCuEoYoxUzDhhSDQ1YQ2pCVVpc452gocFRa9HjyV0FD8UoZ6OvJsCglBzHa8OnzVsxd8GrVnmM0lrb5NuhoNo+0SQwmu58AcvXeB4thDjbbWAjQ0bMw4xFX7VUyzFtXWw7bb2pXdccyTeknMFlj2G9LdIuWyKPD+oYiuMGZjM64rc+LwEFOTZoXtGcyYLCIY8tkuKpzv8BVhndkDe6oEPNK5K5dqTWXa6lVeOE/Y057XOafCx7Nk6WBqL8yKp9o/QTv5DL3+/cfVHo1BDAD7cPupkBxB8VERZymbMhOt+LaDbw5/9F58hhWIXKaA0H2QAWnhLeoQ=',                t: 'MTY3NTUwNTA1Mi4zMTYwMDA=',                m: 'huwkp03FUUZ92f+uj1z1ri+z/UDtSxTEi2kfQAe8UXc=',                i1: 'AOVDmRSzY4mEqw31LLK8pg==',                i2: 'iqndipsrr9TKcGG+p6I0sQ==',                zh: 'qb4aFuGlbJn/rUOkKXjUqElKDKE10jDqu5PE014OTwk=',                uh: 'DV4j3Tmrbi5Rs1q3ahwVS6SgbPbI7np5884QO1u1Cgg=',                hh: 'Ax949TKiHbaXasTISC7ryL1/i3VsF1So3LziNEbpSQM=',            }        };        var trkjs = document.createElement('img');        trkjs.setAttribute('src', '/cdn-cgi/images/trace/captcha/js/transparent.gif?ray=794294b0ff122cc8');        trkjs.setAttribute('alt', '');        trkjs.setAttribute('style', 'display: none');        document.body.appendChild(trkjs);        var cpo = document.createElement('script');        cpo.src = '/cdn-cgi/challenge-platform/h/b/orchestrate/captcha/v1?ray=794294b0ff122cc8';        window._cf_chl_opt.cOgUHash = location.hash === '' && location.href.indexOf('#') !== -1 ? '#' : location.hash;        window._cf_chl_opt.cOgUQuery = location.search === '' && location.href.slice(0, location.href.length - window._cf_chl_opt.cOgUHash.length).indexOf('?') !== -1 ? '?' : location.search;        if (window.history && window.history.replaceState) {            var ogU = location.pathname + window._cf_chl_opt.cOgUQuery + window._cf_chl_opt.cOgUHash;            history.replaceState(null, null, \"\\/api\\/?__cf_chl_rt_tk=3e8YnNWEoJpt7yhj9ZB_z7nPP6BpgWkrEO9fxS_FHfs-1675505052-0-gaNycGzNCv0\" + window._cf_chl_opt.cOgUHash);            cpo.onload = function() {                history.replaceState(null, null, ogU);            };        }        document.getElementsByTagName('head')[0].appendChild(cpo);    }());</script>    <div class=\"footer\" role=\"contentinfo\">        <div class=\"footer-inner\">            <div class=\"clearfix diagnostic-wrapper\">                <div class=\"ray-id\">Ray ID: <code>794294b0ff122cc8</code></div>            </div>            <div class=\"text-center\" id=\"footer-text\">Performance & security by <a rel=\"noopener noreferrer\" href=\"https://www.cloudflare.com?utm_source=challenge&utm_campaign=l\" target=\"_blank\">Cloudflare</a></div>        </div>    </div></body></html>";
        assert!(is_cloudflare_security_challenge(res));
    }

    #[test]
    fn test_cloudflare_response() {
        let resp = "<!DOCTYPE html>\n<!--[if lt IE 7]> <html class=\"no-js ie6 oldie\" lang=\"en-US\"> <![endif]-->\n<!--[if IE 7]>    <html class=\"no-js ie7 oldie\" lang=\"en-US\"> <![endif]-->\n<!--[if IE 8]>    <html class=\"no-js ie8 oldie\" lang=\"en-US\"> <![endif]-->\n<!--[if gt IE 8]><!--> <html class=\"no-js\" lang=\"en-US\"> <!--<![endif]-->\n<head>\n<title>Attention Required! | Cloudflare</title>\n<meta charset=\"UTF-8\" />\n<meta http-equiv=\"Content-Type\" content=\"text/html; charset=UTF-8\" />\n<meta http-equiv=\"X-UA-Compatible\" content=\"IE=Edge\" />\n<meta name=\"robots\" content=\"noindex, nofollow\" />\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\" />\n<link rel=\"stylesheet\" id=\"cf_styles-css\" href=\"/cdn-cgi/styles/cf.errors.css\" />\n<!--[if lt IE 9]><link rel=\"stylesheet\" id='cf_styles-ie-css' href=\"/cdn-cgi/styles/cf.errors.ie.css\" /><![endif]-->\n<style>body{margin:0;padding:0}</style>\n\n\n<!--[if gte IE 10]><!-->\n<script>\n  if (!navigator.cookieEnabled) {\n    window.addEventListener('DOMContentLoaded', function () {\n      var cookieEl = document.getElementById('cookie-alert');\n      cookieEl.style.display = 'block';\n    })\n  }\n</script>\n<!--<![endif]-->\n\n\n</head>\n<body>\n  <div id=\"cf-wrapper\">\n    <div class=\"cf-alert cf-alert-error cf-cookie-error\" id=\"cookie-alert\" data-translate=\"enable_cookies\">Please enable cookies.</div>\n    <div id=\"cf-error-details\" class=\"cf-error-details-wrapper\">\n      <div class=\"cf-wrapper cf-header cf-error-overview\">\n        <h1 data-translate=\"block_headline\">Sorry, you have been blocked</h1>\n        <h2 class=\"cf-subheadline\"><span data-translate=\"unable_to_access\">You are unable to access</span> polygonscan.com</h2>\n      </div><!-- /.header -->\n\n      <div class=\"cf-section cf-highlight\">\n        <div class=\"cf-wrapper\">\n          <div class=\"cf-screenshot-container cf-screenshot-full\">\n            \n              <span class=\"cf-no-screenshot error\"></span>\n            \n          </div>\n        </div>\n      </div><!-- /.captcha-container -->\n\n      <div class=\"cf-section cf-wrapper\">\n        <div class=\"cf-columns two\">\n          <div class=\"cf-column\">\n            <h2 data-translate=\"blocked_why_headline\">Why have I been blocked?</h2>\n\n            <p data-translate=\"blocked_why_detail\">This website is using a security service to protect itself from online attacks. The action you just performed triggered the security solution. There are several actions that could trigger this block including submitting a certain word or phrase, a SQL command or malformed data.</p>\n          </div>\n\n          <div class=\"cf-column\">\n            <h2 data-translate=\"blocked_resolve_headline\">What can I do to resolve this?</h2>\n\n            <p data-translate=\"blocked_resolve_detail\">You can email the site owner to let them know you were blocked. Please include what you were doing when this page came up and the Cloudflare Ray ID found at the bottom of this page.</p>\n          </div>\n        </div>\n      </div><!-- /.section -->\n\n      <div class=\"cf-error-footer cf-wrapper w-240 lg:w-full py-10 sm:py-4 sm:px-8 mx-auto text-center sm:text-left border-solid border-0 border-t border-gray-300\">\n  <p class=\"text-13\">\n    <span class=\"cf-footer-item sm:block sm:mb-1\">Cloudflare Ray ID: <strong class=\"font-semibold\">74d2aa5ed9e27367</strong></span>\n    <span class=\"cf-footer-separator sm:hidden\">&bull;</span>\n    <span id=\"cf-footer-item-ip\" class=\"cf-footer-item hidden sm:block sm:mb-1\">\n      Your IP:\n      <button type=\"button\" id=\"cf-footer-ip-reveal\" class=\"cf-footer-ip-reveal-btn\">Click to reveal</button>\n      <span class=\"hidden\" id=\"cf-footer-ip\">62.96.232.178</span>\n      <span class=\"cf-footer-separator sm:hidden\">&bull;</span>\n    </span>\n    <span class=\"cf-footer-item sm:block sm:mb-1\"><span>Performance &amp; security by</span> <a rel=\"noopener noreferrer\" href=\"https://www.cloudflare.com/5xx-error-landing\" id=\"brand_link\" target=\"_blank\">Cloudflare</a></span>\n    \n  </p>\n  <script>(function(){function d(){var b=a.getElementById(\"cf-footer-item-ip\"),c=a.getElementById(\"cf-footer-ip-reveal\");b&&\"classList\"in b&&(b.classList.remove(\"hidden\"),c.addEventListener(\"click\",function(){c.classList.add(\"hidden\");a.getElementById(\"cf-footer-ip\").classList.remove(\"hidden\")}))}var a=document;document.addEventListener&&a.addEventListener(\"DOMContentLoaded\",d)})();</script>\n</div><!-- /.error-footer -->\n\n\n    </div><!-- /#cf-error-details -->\n  </div><!-- /#cf-wrapper -->\n\n  <script>\n  window._cf_translation = {};\n  \n  \n</script>\n\n</body>\n</html>\n";

        assert!(is_blocked_by_cloudflare_response(resp));
    }
}
