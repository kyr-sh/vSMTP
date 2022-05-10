/*
 * vSMTP mail transfer agent
 * Copyright (C) 2022 viridIT SAS
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License as published by the Free Software
 * Foundation, either version 3 of the License, or any later version.
 *
 * This program is distributed in the hope that it will be useful, but WITHOUT
 * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
 * FOR A PARTICULAR PURPOSE.  See the GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License along with
 * this program. If not, see https://www.gnu.org/licenses/.
 *
*/
use vsmtp_common::{
    auth::Mechanism,
    mail_context::{AuthCredentials, ConnectionContext},
    re::rsasl,
    state::StateSMTP,
    status::{InfoPacket, Status},
};
use vsmtp_config::Config;
use vsmtp_rule_engine::{rule_engine::RuleEngine, rule_state::RuleState};

/// Backend of SASL implementation
pub type Backend = rsasl::DiscardOnDrop<
    rsasl::SASL<
        std::sync::Arc<Config>,
        (
            std::sync::Arc<std::sync::RwLock<RuleEngine>>,
            ConnectionContext,
        ),
    >,
>;

/// SASL session data.
pub type Session = vsmtp_common::re::rsasl::Session<(
    std::sync::Arc<std::sync::RwLock<RuleEngine>>,
    ConnectionContext,
)>;

/// Function called by the SASL backend
pub struct Callback;

impl
    rsasl::Callback<
        std::sync::Arc<Config>,
        (
            std::sync::Arc<std::sync::RwLock<RuleEngine>>,
            ConnectionContext,
        ),
    > for Callback
{
    fn callback(
        sasl: &mut rsasl::SASL<
            std::sync::Arc<Config>,
            (
                std::sync::Arc<std::sync::RwLock<RuleEngine>>,
                ConnectionContext,
            ),
        >,
        session: &mut Session,
        prop: rsasl::Property,
    ) -> Result<(), rsasl::ReturnCode> {
        let config = unsafe { sasl.retrieve() }.ok_or(rsasl::ReturnCode::GSASL_INTEGRITY_ERROR)?;
        sasl.store(config.clone());

        let credentials = match prop {
            rsasl::Property::GSASL_PASSWORD => AuthCredentials::Query {
                authid: session
                    .get_property(rsasl::Property::GSASL_AUTHID)
                    .ok_or(rsasl::ReturnCode::GSASL_NO_AUTHID)?
                    .to_str()
                    .unwrap()
                    .to_string(),
            },
            rsasl::Property::GSASL_VALIDATE_SIMPLE => AuthCredentials::Verify {
                authid: session
                    .get_property(rsasl::Property::GSASL_AUTHID)
                    .ok_or(rsasl::ReturnCode::GSASL_NO_AUTHID)?
                    .to_str()
                    .unwrap()
                    .to_string(),
                authpass: session
                    .get_property(rsasl::Property::GSASL_PASSWORD)
                    .ok_or(rsasl::ReturnCode::GSASL_NO_PASSWORD)?
                    .to_str()
                    .unwrap()
                    .to_string(),
            },
            _ => return Err(rsasl::ReturnCode::GSASL_NO_CALLBACK),
        };

        let (rule_engine, conn) = session
            .retrieve_mut()
            .ok_or(rsasl::ReturnCode::GSASL_INTEGRITY_ERROR)?;

        let mut conn = conn.clone();
        conn.credentials = Some(credentials);

        let result = {
            let re = rule_engine
                .read()
                .map_err(|_| rsasl::ReturnCode::GSASL_INTEGRITY_ERROR)?;

            let mut rule_state = RuleState::with_connection(&config, &re, conn);

            re.run_when(
                &mut rule_state,
                &StateSMTP::Authentication(Mechanism::default(), None),
            )
        };

        match prop {
            rsasl::Property::GSASL_VALIDATE_SIMPLE if result == Status::Accept => Ok(()),
            rsasl::Property::GSASL_PASSWORD => {
                let authpass = match result {
                    Status::Info(InfoPacket::Str(authpass)) => authpass,
                    _ => return Err(rsasl::ReturnCode::GSASL_AUTHENTICATION_ERROR),
                };

                session.set_property(rsasl::Property::GSASL_PASSWORD, authpass.as_bytes());
                Ok(())
            }
            _ => Err(rsasl::ReturnCode::GSASL_AUTHENTICATION_ERROR),
        }
    }
}