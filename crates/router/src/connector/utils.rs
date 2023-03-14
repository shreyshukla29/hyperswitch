use std::collections::HashMap;

use common_utils::pii;
use error_stack::{report, IntoReport, ResultExt};
use masking::Secret;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::{
    core::errors::{self, CustomResult},
    pii::PeekInterface,
    types::{self, api, PaymentsCancelData},
    utils::OptionExt,
};

pub fn missing_field_err(
    message: &'static str,
) -> Box<dyn Fn() -> error_stack::Report<errors::ConnectorError> + '_> {
    Box::new(move || {
        errors::ConnectorError::MissingRequiredField {
            field_name: message,
        }
        .into()
    })
}

type Error = error_stack::Report<errors::ConnectorError>;

pub trait AccessTokenRequestInfo {
    fn get_request_id(&self) -> Result<String, Error>;
}

impl AccessTokenRequestInfo for types::RefreshTokenRouterData {
    fn get_request_id(&self) -> Result<String, Error> {
        self.request
            .id
            .clone()
            .ok_or_else(missing_field_err("request.id"))
    }
}

pub trait RouterData {
    fn get_billing(&self) -> Result<&api::Address, Error>;
    fn get_billing_country(&self) -> Result<String, Error>;
    fn get_billing_phone(&self) -> Result<&api::PhoneDetails, Error>;
    fn get_description(&self) -> Result<String, Error>;
    fn get_billing_address(&self) -> Result<&api::AddressDetails, Error>;
    fn get_connector_meta(&self) -> Result<serde_json::Value, Error>;
    fn get_session_token(&self) -> Result<String, Error>;
    fn to_connector_meta<T>(&self) -> Result<T, Error>
    where
        T: serde::de::DeserializeOwned;
    fn get_return_url(&self) -> Result<String, Error>;
}

impl<Flow, Request, Response> RouterData for types::RouterData<Flow, Request, Response> {
    fn get_billing(&self) -> Result<&api::Address, Error> {
        self.address
            .billing
            .as_ref()
            .ok_or_else(missing_field_err("billing"))
    }

    fn get_billing_country(&self) -> Result<String, Error> {
        self.address
            .billing
            .as_ref()
            .and_then(|a| a.address.as_ref())
            .and_then(|ad| ad.country.clone())
            .ok_or_else(missing_field_err("billing.address.country"))
    }

    fn get_billing_phone(&self) -> Result<&api::PhoneDetails, Error> {
        self.address
            .billing
            .as_ref()
            .and_then(|a| a.phone.as_ref())
            .ok_or_else(missing_field_err("billing.phone"))
    }
    fn get_description(&self) -> Result<String, Error> {
        self.description
            .clone()
            .ok_or_else(missing_field_err("description"))
    }
    fn get_billing_address(&self) -> Result<&api::AddressDetails, Error> {
        self.address
            .billing
            .as_ref()
            .and_then(|a| a.address.as_ref())
            .ok_or_else(missing_field_err("billing.address"))
    }
    fn get_connector_meta(&self) -> Result<serde_json::Value, Error> {
        self.connector_meta_data
            .clone()
            .ok_or_else(missing_field_err("connector_meta_data"))
    }

    fn get_session_token(&self) -> Result<String, Error> {
        self.session_token
            .clone()
            .ok_or_else(missing_field_err("session_token"))
    }

    fn to_connector_meta<T>(&self) -> Result<T, Error>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_value::<T>(self.get_connector_meta()?)
            .into_report()
            .change_context(errors::ConnectorError::NoConnectorMetaData)
    }

    fn get_return_url(&self) -> Result<String, Error> {
        self.router_return_url
            .clone()
            .ok_or_else(missing_field_err("return_url"))
    }
}

pub trait PaymentsRequestData {
    fn get_card(&self) -> Result<api::Card, Error>;
}

impl PaymentsRequestData for types::PaymentsAuthorizeRouterData {
    fn get_card(&self) -> Result<api::Card, Error> {
        match self.request.payment_method_data.clone() {
            api::PaymentMethodData::Card(card) => Ok(card),
            _ => Err(missing_field_err("card")()),
        }
    }
}

pub trait PaymentsAuthorizeRequestData {
    fn is_auto_capture(&self) -> bool;
}

impl PaymentsAuthorizeRequestData for types::PaymentsAuthorizeData {
    fn is_auto_capture(&self) -> bool {
        self.capture_method == Some(storage_models::enums::CaptureMethod::Automatic)
    }
}

pub trait PaymentsSyncRequestData {
    fn is_auto_capture(&self) -> bool;
}

impl PaymentsSyncRequestData for types::PaymentsSyncData {
    fn is_auto_capture(&self) -> bool {
        self.capture_method == Some(storage_models::enums::CaptureMethod::Automatic)
    }
}

pub trait PaymentsCancelRequestData {
    fn get_amount(&self) -> Result<i64, Error>;
    fn get_currency(&self) -> Result<storage_models::enums::Currency, Error>;
}

impl PaymentsCancelRequestData for PaymentsCancelData {
    fn get_amount(&self) -> Result<i64, Error> {
        self.amount.ok_or_else(missing_field_err("amount"))
    }
    fn get_currency(&self) -> Result<storage_models::enums::Currency, Error> {
        self.currency.ok_or_else(missing_field_err("currency"))
    }
}

pub trait RefundsRequestData {
    fn get_connector_refund_id(&self) -> Result<String, Error>;
}

impl RefundsRequestData for types::RefundsData {
    fn get_connector_refund_id(&self) -> Result<String, Error> {
        self.connector_refund_id
            .clone()
            .get_required_value("connector_refund_id")
            .change_context(errors::ConnectorError::MissingConnectorTransactionID)
    }
}

static CARD_REGEX: Lazy<HashMap<CardIssuer, Result<Regex, regex::Error>>> = Lazy::new(|| {
    let mut map = HashMap::new();
    // Reference: https://gist.github.com/michaelkeevildown/9096cd3aac9029c4e6e05588448a8841
    // [#379]: Determine card issuer from card BIN number
    map.insert(CardIssuer::Master, Regex::new(r"^5[1-5][0-9]{14}$"));
    map.insert(CardIssuer::AmericanExpress, Regex::new(r"^3[47][0-9]{13}$"));
    map.insert(CardIssuer::Visa, Regex::new(r"^4[0-9]{12}(?:[0-9]{3})?$"));
    map.insert(CardIssuer::Discover, Regex::new(r"^65[4-9][0-9]{13}|64[4-9][0-9]{13}|6011[0-9]{12}|(622(?:12[6-9]|1[3-9][0-9]|[2-8][0-9][0-9]|9[01][0-9]|92[0-5])[0-9]{10})$"));
    map.insert(
        CardIssuer::Maestro,
        Regex::new(r"^(5018|5020|5038|5893|6304|6759|6761|6762|6763)[0-9]{8,15}$"),
    );
    map
});

#[derive(Debug, Copy, Clone, strum::Display, Eq, Hash, PartialEq)]
pub enum CardIssuer {
    AmericanExpress,
    Master,
    Maestro,
    Visa,
    Discover,
}

pub trait CardData {
    fn get_card_expiry_year_2_digit(&self) -> Secret<String>;
    fn get_card_issuer(&self) -> Result<CardIssuer, Error>;
    fn get_expiry_date_as_yyyymm(&self, delimiter: &str) -> String;
}

impl CardData for api::Card {
    fn get_card_expiry_year_2_digit(&self) -> Secret<String> {
        let binding = self.card_exp_year.clone();
        let year = binding.peek();
        Secret::new(year[year.len() - 2..].to_string())
    }
    fn get_card_issuer(&self) -> Result<CardIssuer, Error> {
        let card: Secret<String, pii::CardNumber> = self
            .card_number
            .clone()
            .map(|card| card.split_whitespace().collect());
        get_card_issuer(card.peek().clone().as_str())
    }
    fn get_expiry_date_as_yyyymm(&self, delimiter: &str) -> String {
        format!(
            "{}{}{}",
            self.card_exp_year.peek().clone(),
            delimiter,
            self.card_exp_month.peek().clone()
        )
    }
}

fn get_card_issuer(card_number: &str) -> Result<CardIssuer, Error> {
    for (k, v) in CARD_REGEX.iter() {
        let regex: Regex = v
            .clone()
            .into_report()
            .change_context(errors::ConnectorError::RequestEncodingFailed)?;
        if regex.is_match(card_number) {
            return Ok(*k);
        }
    }
    Err(error_stack::Report::new(
        errors::ConnectorError::NotImplemented("Card Type".into()),
    ))
}
pub trait PhoneDetailsData {
    fn get_number(&self) -> Result<Secret<String>, Error>;
    fn get_country_code(&self) -> Result<String, Error>;
}

impl PhoneDetailsData for api::PhoneDetails {
    fn get_country_code(&self) -> Result<String, Error> {
        self.country_code
            .clone()
            .ok_or_else(missing_field_err("billing.phone.country_code"))
    }
    fn get_number(&self) -> Result<Secret<String>, Error> {
        self.number
            .clone()
            .ok_or_else(missing_field_err("billing.phone.number"))
    }
}

pub trait AddressDetailsData {
    fn get_first_name(&self) -> Result<&Secret<String>, Error>;
    fn get_last_name(&self) -> Result<&Secret<String>, Error>;
    fn get_line1(&self) -> Result<&Secret<String>, Error>;
    fn get_city(&self) -> Result<&String, Error>;
    fn get_line2(&self) -> Result<&Secret<String>, Error>;
    fn get_zip(&self) -> Result<&Secret<String>, Error>;
    fn get_country(&self) -> Result<&String, Error>;
}

impl AddressDetailsData for api::AddressDetails {
    fn get_first_name(&self) -> Result<&Secret<String>, Error> {
        self.first_name
            .as_ref()
            .ok_or_else(missing_field_err("address.first_name"))
    }

    fn get_last_name(&self) -> Result<&Secret<String>, Error> {
        self.last_name
            .as_ref()
            .ok_or_else(missing_field_err("address.last_name"))
    }

    fn get_line1(&self) -> Result<&Secret<String>, Error> {
        self.line1
            .as_ref()
            .ok_or_else(missing_field_err("address.line1"))
    }

    fn get_city(&self) -> Result<&String, Error> {
        self.city
            .as_ref()
            .ok_or_else(missing_field_err("address.city"))
    }

    fn get_line2(&self) -> Result<&Secret<String>, Error> {
        self.line2
            .as_ref()
            .ok_or_else(missing_field_err("address.line2"))
    }

    fn get_zip(&self) -> Result<&Secret<String>, Error> {
        self.zip
            .as_ref()
            .ok_or_else(missing_field_err("address.zip"))
    }

    fn get_country(&self) -> Result<&String, Error> {
        self.country
            .as_ref()
            .ok_or_else(missing_field_err("address.country"))
    }
}

pub fn get_header_key_value<'a>(
    key: &str,
    headers: &'a actix_web::http::header::HeaderMap,
) -> CustomResult<&'a str, errors::ConnectorError> {
    headers
        .get(key)
        .map(|header_value| {
            header_value
                .to_str()
                .into_report()
                .change_context(errors::ConnectorError::WebhookSignatureNotFound)
        })
        .ok_or(report!(
            errors::ConnectorError::WebhookSourceVerificationFailed
        ))?
}

pub fn to_currency_base_unit_from_optional_amount(
    amount: Option<i64>,
    currency: storage_models::enums::Currency,
) -> Result<String, error_stack::Report<errors::ConnectorError>> {
    match amount {
        Some(a) => to_currency_base_unit(a, currency),
        _ => Err(errors::ConnectorError::MissingRequiredField {
            field_name: "amount",
        }
        .into()),
    }
}

pub fn to_currency_base_unit(
    amount: i64,
    currency: storage_models::enums::Currency,
) -> Result<String, error_stack::Report<errors::ConnectorError>> {
    let amount_u32 = u32::try_from(amount)
        .into_report()
        .change_context(errors::ConnectorError::RequestEncodingFailed)?;
    match currency {
        storage_models::enums::Currency::JPY | storage_models::enums::Currency::KRW => {
            Ok(amount.to_string())
        }
        storage_models::enums::Currency::BHD
        | storage_models::enums::Currency::JOD
        | storage_models::enums::Currency::KWD
        | storage_models::enums::Currency::OMR => Ok((f64::from(amount_u32) / 1000.0).to_string()),
        _ => Ok((f64::from(amount_u32) / 100.0).to_string()),
    }
}

pub fn to_connector_meta<T>(connector_meta: Option<serde_json::Value>) -> Result<T, Error>
where
    T: serde::de::DeserializeOwned,
{
    let json = connector_meta.ok_or_else(missing_field_err("connector_meta_data"))?;
    parse_struct(json)
}
pub fn parse_struct<T>(json: serde_json::Value) -> Result<T, Error>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value::<T>(json.clone())
        .into_report()
        .change_context(errors::ConnectorError::ParsingFailed {
            from_type: "Json",
            to_type: std::any::type_name::<T>(),
            data: json.to_string(),
        })
}
