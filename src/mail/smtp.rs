//! Delivery over SMTP with verified TLS only (mail-outbox: SMTP delivery requires
//! verified TLS; change foundation D17). `TlsParameters` are built offline at startup.

use std::time::Duration;

use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{
    Certificate, CertificateStore, Tls, TlsParameters, TlsVersion,
};
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use super::{Failure, FailureClass, Mailer, Outgoing, SendFuture};
use crate::config::{DnsName, SmtpConfig, SmtpTls};

/// Time one SMTP command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// The CA roots the certificate must chain to.
pub struct SmtpRoots(Roots);

enum Roots {
    Compiled,
    /// Only `tests/smtp_tls.rs` uses this; the source scan keeps it out of `src/`.
    Extra(Vec<u8>),
}

impl SmtpRoots {
    /// The public CA roots compiled into the binary (webpki-roots), never the system's.
    pub fn compiled() -> SmtpRoots {
        SmtpRoots(Roots::Compiled)
    }

    /// Exactly one test CA, PEM-encoded.
    pub fn for_tests(ca_pem: &[u8]) -> SmtpRoots {
        SmtpRoots(Roots::Extra(ca_pem.to_vec()))
    }
}

/// TLS settings could not be built (a malformed test CA); never happens with the
/// compiled roots.
#[derive(Debug)]
pub struct SmtpSetupError;

impl std::fmt::Display for SmtpSetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cannot set up SMTP over TLS")
    }
}

impl std::error::Error for SmtpSetupError {}

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpMailer {
    /// Builds the transport without contacting the server. `ehlo` is the main host.
    pub fn new(
        config: &SmtpConfig,
        ehlo: &DnsName,
        roots: SmtpRoots,
    ) -> Result<SmtpMailer, SmtpSetupError> {
        let host = config.host.as_str().to_owned();
        let builder = TlsParameters::builder(host.clone()).set_min_tls_version(TlsVersion::Tlsv12);
        let builder = match roots.0 {
            Roots::Compiled => builder.certificate_store(CertificateStore::WebpkiRoots),
            Roots::Extra(pem) => builder
                .certificate_store(CertificateStore::None)
                .add_root_certificate(Certificate::from_pem(&pem).map_err(|_| SmtpSetupError)?),
        };
        let parameters = builder.build_rustls().map_err(|_| SmtpSetupError)?;
        let tls = match config.tls {
            SmtpTls::Implicit => Tls::Wrapper(parameters),
            SmtpTls::Starttls => Tls::Required(parameters),
        };
        let transport = AsyncSmtpTransport::<Tokio1Executor>::relay(&host)
            .map_err(|_| SmtpSetupError)?
            .port(config.port)
            .tls(tls)
            .credentials(Credentials::new(
                config.username.clone(),
                config.password.expose().to_owned(),
            ))
            .hello_name(ClientId::Domain(ehlo.as_str().to_owned()))
            .timeout(Some(COMMAND_TIMEOUT))
            .build();
        Ok(SmtpMailer { transport })
    }
}

impl Mailer for SmtpMailer {
    fn send(&self, outgoing: Outgoing) -> SendFuture<'_> {
        Box::pin(async move {
            self.transport
                .send(outgoing.message)
                .await
                .map(drop)
                .map_err(|error| classify(&error))
        })
    }
}

/// Reduces a lettre error to a fixed class and reply code; its text, which can quote
/// addresses or the server's reply, is dropped.
pub fn classify(error: &lettre::transport::smtp::Error) -> Failure {
    let code = error.status().map(u16::from);
    let class = if error.is_timeout() {
        FailureClass::Timeout
    } else if error.is_tls() {
        FailureClass::Tls
    } else if matches!(code, Some(530 | 534 | 535 | 538)) {
        FailureClass::Authentication
    } else if error.is_permanent() {
        FailureClass::Rejected
    } else if error.is_transient() {
        FailureClass::Temporary
    } else if error.is_client() || error.is_response() {
        FailureClass::Protocol
    } else {
        FailureClass::Connection
    };
    Failure { class, code }
}
