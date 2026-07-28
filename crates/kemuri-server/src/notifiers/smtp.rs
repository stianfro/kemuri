use async_trait::async_trait;
use kemuri_core::NotifierId;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use super::{NotificationError, NotificationPayload, NotificationResult, Notifier, resolve_secret};

pub struct SmtpNotifier {
    id: NotifierId,
    from: String,
    to: Vec<String>,
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpNotifier {
    pub fn from_config(
        params: &kemuri_config::SmtpNotifierParams,
    ) -> Result<Self, NotificationError> {
        let timeout = kemuri_core::parse_duration(&params.timeout)
            .unwrap_or(std::time::Duration::from_secs(30));

        let mut builder = match params.tls_mode.as_str() {
            "disabled" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&params.host)
                .port(params.port)
                .timeout(Some(timeout)),
            _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&params.host)
                .map_err(|e| {
                    NotificationError::Config(format!("SMTP transport creation failed: {}", e))
                })?
                .port(params.port)
                .timeout(Some(timeout)),
        };

        if let Some(ref username) = params.username {
            let password = params
                .password
                .as_ref()
                .map(resolve_secret)
                .transpose()?
                .unwrap_or_default();
            let creds = Credentials::new(username.clone(), password);
            builder = builder.credentials(creds);
        }

        Ok(Self {
            id: params.id.clone(),
            from: params.from.clone(),
            to: params.to.clone(),
            transport: builder.build(),
        })
    }
}

#[async_trait]
impl Notifier for SmtpNotifier {
    fn kind(&self) -> &str {
        "smtp"
    }

    fn id(&self) -> &NotifierId {
        &self.id
    }

    async fn send(&self, notification: NotificationPayload) -> NotificationResult {
        let subject = match notification.event_type {
            kemuri_core::AlertEventKind::Firing => {
                format!(
                    "[FIRING] {} - {}/{}",
                    notification.rule_id, notification.target_name, notification.check_id
                )
            }
            kemuri_core::AlertEventKind::Resolved => {
                format!(
                    "[RESOLVED] {} - {}/{}",
                    notification.rule_id, notification.target_name, notification.check_id
                )
            }
        };

        let plain_body = format!(
            "{}\n\nRule: {}\nTarget: {} ({})\nCheck: {}\nProbe: {}\nCurrent value: {}\nThreshold: {}\nState since: {}\nEvent time: {}\n{}",
            notification.summary,
            notification.rule_id,
            notification.target_name,
            notification.target_id,
            notification.check_id,
            notification.probe_type,
            notification.current_value,
            notification.threshold,
            notification.state_start_time.to_rfc3339(),
            notification.event_time.to_rfc3339(),
            notification.kemuri_url.as_deref().unwrap_or(""),
        );

        let html_body = format!(
            "<html><body><h2>{}</h2><table><tr><td>Rule</td><td>{}</td></tr><tr><td>Target</td><td>{} ({})</td></tr><tr><td>Check</td><td>{}</td></tr><tr><td>Probe</td><td>{}</td></tr><tr><td>Current value</td><td>{}</td></tr><tr><td>Threshold</td><td>{}</td></tr><tr><td>State since</td><td>{}</td></tr><tr><td>Event time</td><td>{}</td></tr></table>{}</body></html>",
            notification.summary,
            notification.rule_id,
            notification.target_name,
            notification.target_id,
            notification.check_id,
            notification.probe_type,
            notification.current_value,
            notification.threshold,
            notification.state_start_time.to_rfc3339(),
            notification.event_time.to_rfc3339(),
            notification
                .kemuri_url
                .as_deref()
                .map(|u| format!("<p><a href=\"{}\">View in Kemuri</a></p>", u))
                .unwrap_or_default(),
        );

        for recipient in &self.to {
            let email = Message::builder()
                .from(self.from.parse().map_err(|e| {
                    NotificationError::Config(format!("invalid from address: {}", e))
                })?)
                .to(recipient
                    .parse()
                    .map_err(|e| NotificationError::Config(format!("invalid to address: {}", e)))?)
                .subject(&subject)
                .multipart(
                    lettre::message::MultiPart::alternative()
                        .singlepart(
                            lettre::message::SinglePart::builder()
                                .header(ContentType::TEXT_PLAIN)
                                .body(plain_body.clone()),
                        )
                        .singlepart(
                            lettre::message::SinglePart::builder()
                                .header(ContentType::TEXT_HTML)
                                .body(html_body.clone()),
                        ),
                )
                .map_err(|e| NotificationError::Config(format!("failed to build email: {}", e)))?;

            self.transport
                .send(email)
                .await
                .map_err(|e| NotificationError::Delivery(format!("SMTP send failed: {}", e)))?;
        }

        Ok(())
    }
}
