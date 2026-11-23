//! No DTDs, external entities, nested arguments or SOAP-encoded objects cross this boundary.
use super::{escaped, xml_character, TvBrowse, TvCatalog, TvError, TvService};
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use std::{collections::BTreeMap, net::SocketAddr};
const SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";
impl TvService {
    pub fn urn(self) -> &'static str {
        match self {
            Self::ContentDirectory => "urn:schemas-upnp-org:service:ContentDirectory:1",
            Self::ConnectionManager => "urn:schemas-upnp-org:service:ConnectionManager:1",
        }
    }
}
impl TvError {
    pub fn code(self) -> u16 {
        match self {
            Self::Invalid => 402,
            Self::Missing => 701,
            Self::NotContainer => 710,
            Self::SortUnsupported => 709,
            Self::Action => 401,
            Self::ConnectionMissing => 706,
        }
    }
}
impl TvCatalog {
    pub fn control(
        &self,
        address: SocketAddr,
        service: TvService,
        header: &str,
        body: &[u8],
    ) -> Result<String, TvError> {
        if address.port() == 0 || super::super::lan::validate_bind(address.ip()).is_err() {
            return Err(TvError::Invalid);
        }
        let (action, args) = parse(service, header, body)?;
        let fields = match (service, action.as_str()) {
            (TvService::ContentDirectory, "Browse") => {
                exact(
                    &args,
                    &[
                        "ObjectID",
                        "BrowseFlag",
                        "Filter",
                        "StartingIndex",
                        "RequestedCount",
                        "SortCriteria",
                    ],
                )?;
                let metadata = match args["BrowseFlag"].as_str() {
                    "BrowseMetadata" => true,
                    "BrowseDirectChildren" => false,
                    _ => return Err(TvError::Invalid),
                };
                let page = self.browse_filtered(
                    address,
                    TvBrowse {
                        object_id: &args["ObjectID"],
                        metadata,
                        start: number(&args["StartingIndex"])?,
                        count: number(&args["RequestedCount"])?,
                        sort: &args["SortCriteria"],
                    },
                    &args["Filter"],
                )?;
                vec![
                    ("Result", page.xml),
                    ("NumberReturned", page.returned.to_string()),
                    ("TotalMatches", page.total.to_string()),
                    ("UpdateID", "0".into()),
                ]
            }
            (TvService::ContentDirectory, "GetSearchCapabilities") => {
                exact(&args, &[])?;
                vec![("SearchCaps", String::new())]
            }
            (TvService::ContentDirectory, "GetSortCapabilities") => {
                exact(&args, &[])?;
                vec![("SortCaps", "dc:title".into())]
            }
            (TvService::ContentDirectory, "GetSystemUpdateID") => {
                exact(&args, &[])?;
                vec![("Id", "0".into())]
            }
            (TvService::ConnectionManager, "GetProtocolInfo") => {
                exact(&args, &[])?;
                vec![("Source", self.protocol_info()), ("Sink", String::new())]
            }
            (TvService::ConnectionManager, "GetCurrentConnectionIDs") => {
                exact(&args, &[])?;
                vec![("ConnectionIDs", "0".into())]
            }
            (TvService::ConnectionManager, "GetCurrentConnectionInfo") => {
                exact(&args, &["ConnectionID"])?;
                if args["ConnectionID"]
                    .parse::<i32>()
                    .map_err(|_| TvError::Invalid)?
                    != 0
                {
                    return Err(TvError::ConnectionMissing);
                }
                vec![
                    ("RcsID", "-1".into()),
                    ("AVTransportID", "-1".into()),
                    ("ProtocolInfo", String::new()),
                    ("PeerConnectionManager", String::new()),
                    ("PeerConnectionID", "-1".into()),
                    ("Direction", "Output".into()),
                    ("Status", "OK".into()),
                ]
            }
            _ => return Err(TvError::Action),
        };
        let fields = fields
            .into_iter()
            .map(|(name, value)| format!("<{name}>{}</{name}>", escaped(&value)))
            .collect::<String>();
        Ok(format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"{SOAP}\"><s:Body><u:{action}Response xmlns:u=\"{}\">{fields}</u:{action}Response></s:Body></s:Envelope>",service.urn()))
    }
}
fn exact(args: &BTreeMap<String, String>, names: &[&str]) -> Result<(), TvError> {
    if args.len() != names.len() || names.iter().any(|name| !args.contains_key(*name)) {
        Err(TvError::Invalid)
    } else {
        Ok(())
    }
}
fn number(value: &str) -> Result<u32, TvError> {
    if value.is_empty() || !value.bytes().all(|value| value.is_ascii_digit()) {
        return Err(TvError::Invalid);
    }
    value.parse().map_err(|_| TvError::Invalid)
}
fn parse(
    service: TvService,
    header: &str,
    body: &[u8],
) -> Result<(String, BTreeMap<String, String>), TvError> {
    if body.len() > 16 * 1024 || header.len() > 256 {
        return Err(TvError::Invalid);
    }
    let body = std::str::from_utf8(body).map_err(|_| TvError::Invalid)?;
    if !body.chars().all(xml_character) {
        return Err(TvError::Invalid);
    }
    let header = header.trim();
    let header = header
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(header);
    let (urn, action) = header.split_once('#').ok_or(TvError::Invalid)?;
    if urn != service.urn()
        || action.is_empty()
        || !action
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || v == b'_')
    {
        return Err(TvError::Invalid);
    }
    let mut reader = NsReader::from_str(body);
    reader.config_mut().expand_empty_elements = true;
    let mut path = Vec::new();
    let (mut roots, mut bodies, mut actions) = (0, 0, 0);
    let mut args = BTreeMap::new();
    for _ in 0..256 {
        match reader.read_event().map_err(|_| TvError::Invalid)? {
            Event::Start(element) => {
                let namespace = match reader.resolver().resolve_element(element.name()).0 {
                    ResolveResult::Bound(value) => value.as_ref().to_owned(),
                    ResolveResult::Unbound => String::new(),
                    _ => return Err(TvError::Invalid),
                };
                let name = element.local_name().as_ref().to_owned();
                let mut attributes = 0;
                for attr in element.attributes() {
                    let attr = attr.map_err(|_| TvError::Invalid)?;
                    attributes += 1;
                    if attributes > 16 {
                        return Err(TvError::Invalid);
                    }
                    if path.len() >= 2
                        && attr.key.as_ref() != "xmlns"
                        && !attr.key.as_ref().starts_with("xmlns:")
                    {
                        return Err(TvError::Invalid);
                    }
                }
                match path.len() {
                    0 => {
                        roots += 1;
                        if roots != 1 || name != "Envelope" || namespace != SOAP {
                            return Err(TvError::Invalid);
                        }
                    }
                    1 => {
                        bodies += 1;
                        if bodies != 1 || name != "Body" || namespace != SOAP {
                            return Err(TvError::Invalid);
                        }
                    }
                    2 => {
                        actions += 1;
                        if actions != 1 || name != action || namespace != urn {
                            return Err(TvError::Invalid);
                        }
                    }
                    3 => {
                        if (!namespace.is_empty() && namespace != urn)
                            || args.len() >= 8
                            || args.insert(name.clone(), String::new()).is_some()
                        {
                            return Err(TvError::Invalid);
                        }
                    }
                    _ => return Err(TvError::Invalid),
                }
                path.push(name);
            }
            Event::End(_) => {
                if path.pop().is_none() {
                    return Err(TvError::Invalid);
                }
            }
            Event::Text(text) => append(&path, &mut args, &text.xml10_content())?,
            Event::CData(text) => append(&path, &mut args, &text.xml10_content())?,
            Event::GeneralRef(reference) => {
                let value = if let Some(value) =
                    reference.resolve_char_ref().map_err(|_| TvError::Invalid)?
                {
                    value
                } else {
                    match reference.as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return Err(TvError::Invalid),
                    }
                };
                if !xml_character(value) {
                    return Err(TvError::Invalid);
                }
                append(&path, &mut args, &value.to_string())?;
            }
            Event::Decl(_) if path.is_empty() && roots == 0 => {}
            Event::Comment(_) => {}
            Event::Eof => {
                if !path.is_empty() || roots != 1 || bodies != 1 || actions != 1 {
                    return Err(TvError::Invalid);
                }
                return Ok((action.to_owned(), args));
            }
            _ => return Err(TvError::Invalid),
        }
    }
    Err(TvError::Invalid)
}
fn append(path: &[String], args: &mut BTreeMap<String, String>, text: &str) -> Result<(), TvError> {
    if path.len() != 4 {
        return if text.trim().is_empty() {
            Ok(())
        } else {
            Err(TvError::Invalid)
        };
    }
    let name = path.last().ok_or(TvError::Invalid)?;
    let value = args.get_mut(name).ok_or(TvError::Invalid)?;
    if value.len() + text.len() > 512 {
        return Err(TvError::Invalid);
    }
    value.push_str(text);
    Ok(())
}
