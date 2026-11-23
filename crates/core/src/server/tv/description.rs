//! Original descriptor generation from the UPnP AV service schemas; no copied server code.
use super::TvService;
pub(super) fn device(uuid: &str) -> String {
    let mut services = String::new();
    for (service, id, path) in [
        (
            TvService::ContentDirectory,
            "ContentDirectory",
            "content-directory",
        ),
        (
            TvService::ConnectionManager,
            "ConnectionManager",
            "connection-manager",
        ),
    ] {
        services.push_str(&format!("<service><serviceType>{}</serviceType><serviceId>urn:upnp-org:serviceId:{id}</serviceId><SCPDURL>/{path}.xml</SCPDURL><controlURL>/control/{path}</controlURL><eventSubURL>/events/{path}</eventSubURL></service>",service.urn()));
    }
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><root xmlns=\"urn:schemas-upnp-org:device-1-0\"><specVersion><major>1</major><minor>0</minor></specVersion><device><deviceType>urn:schemas-upnp-org:device:MediaServer:1</deviceType><friendlyName>Tonalli TV</friendlyName><manufacturer>Tonalli project</manufacturer><modelName>Tonalli</modelName><UDN>uuid:{uuid}</UDN><serviceList>{services}</serviceList></device></root>")
}
pub(super) fn service(service: TvService) -> String {
    let mut actions = String::new();
    let mut variables = String::new();
    match service {
        TvService::ContentDirectory => {
            action(
                &mut actions,
                "GetSearchCapabilities",
                &[("SearchCaps", "out", "SearchCapabilities")],
            );
            action(
                &mut actions,
                "GetSortCapabilities",
                &[("SortCaps", "out", "SortCapabilities")],
            );
            action(
                &mut actions,
                "GetSystemUpdateID",
                &[("Id", "out", "SystemUpdateID")],
            );
            action(
                &mut actions,
                "Browse",
                &[
                    ("ObjectID", "in", "A_ARG_TYPE_ObjectID"),
                    ("BrowseFlag", "in", "A_ARG_TYPE_BrowseFlag"),
                    ("Filter", "in", "A_ARG_TYPE_Filter"),
                    ("StartingIndex", "in", "A_ARG_TYPE_Index"),
                    ("RequestedCount", "in", "A_ARG_TYPE_Count"),
                    ("SortCriteria", "in", "A_ARG_TYPE_SortCriteria"),
                    ("Result", "out", "A_ARG_TYPE_Result"),
                    ("NumberReturned", "out", "A_ARG_TYPE_Count"),
                    ("TotalMatches", "out", "A_ARG_TYPE_Count"),
                    ("UpdateID", "out", "A_ARG_TYPE_UpdateID"),
                ],
            );
            for (name, kind, event) in [
                ("SearchCapabilities", "string", false),
                ("SortCapabilities", "string", false),
                ("SystemUpdateID", "ui4", true),
                ("A_ARG_TYPE_ObjectID", "string", false),
                ("A_ARG_TYPE_BrowseFlag", "string", false),
                ("A_ARG_TYPE_Filter", "string", false),
                ("A_ARG_TYPE_Index", "ui4", false),
                ("A_ARG_TYPE_Count", "ui4", false),
                ("A_ARG_TYPE_SortCriteria", "string", false),
                ("A_ARG_TYPE_Result", "string", false),
                ("A_ARG_TYPE_UpdateID", "ui4", false),
            ] {
                variable(&mut variables, name, kind, event);
            }
        }
        TvService::ConnectionManager => {
            action(
                &mut actions,
                "GetProtocolInfo",
                &[
                    ("Source", "out", "SourceProtocolInfo"),
                    ("Sink", "out", "SinkProtocolInfo"),
                ],
            );
            action(
                &mut actions,
                "GetCurrentConnectionIDs",
                &[("ConnectionIDs", "out", "CurrentConnectionIDs")],
            );
            action(
                &mut actions,
                "GetCurrentConnectionInfo",
                &[
                    ("ConnectionID", "in", "A_ARG_TYPE_ConnectionID"),
                    ("RcsID", "out", "A_ARG_TYPE_RcsID"),
                    ("AVTransportID", "out", "A_ARG_TYPE_AVTransportID"),
                    ("ProtocolInfo", "out", "A_ARG_TYPE_ProtocolInfo"),
                    (
                        "PeerConnectionManager",
                        "out",
                        "A_ARG_TYPE_ConnectionManager",
                    ),
                    ("PeerConnectionID", "out", "A_ARG_TYPE_ConnectionID"),
                    ("Direction", "out", "A_ARG_TYPE_Direction"),
                    ("Status", "out", "A_ARG_TYPE_ConnectionStatus"),
                ],
            );
            for (name, kind, event) in [
                ("SourceProtocolInfo", "string", true),
                ("SinkProtocolInfo", "string", true),
                ("CurrentConnectionIDs", "string", true),
                ("A_ARG_TYPE_ConnectionID", "i4", false),
                ("A_ARG_TYPE_RcsID", "i4", false),
                ("A_ARG_TYPE_AVTransportID", "i4", false),
                ("A_ARG_TYPE_ProtocolInfo", "string", false),
                ("A_ARG_TYPE_ConnectionManager", "string", false),
                ("A_ARG_TYPE_Direction", "string", false),
                ("A_ARG_TYPE_ConnectionStatus", "string", false),
            ] {
                variable(&mut variables, name, kind, event);
            }
        }
    }
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><scpd xmlns=\"urn:schemas-upnp-org:service-1-0\"><specVersion><major>1</major><minor>0</minor></specVersion><actionList>{actions}</actionList><serviceStateTable>{variables}</serviceStateTable></scpd>")
}
fn action(xml: &mut String, name: &str, args: &[(&str, &str, &str)]) {
    xml.push_str(&format!("<action><name>{name}</name>"));
    if !args.is_empty() {
        xml.push_str("<argumentList>");
        for (name, direction, variable) in args {
            xml.push_str(&format!("<argument><name>{name}</name><direction>{direction}</direction><relatedStateVariable>{variable}</relatedStateVariable></argument>"));
        }
        xml.push_str("</argumentList>");
    }
    xml.push_str("</action>");
}
fn variable(xml: &mut String, name: &str, kind: &str, event: bool) {
    let event = if event { "yes" } else { "no" };
    xml.push_str(&format!(
        "<stateVariable sendEvents=\"{event}\"><name>{name}</name><dataType>{kind}</dataType>"
    ));
    let values: &[&str] = match name {
        "A_ARG_TYPE_BrowseFlag" => &["BrowseMetadata", "BrowseDirectChildren"],
        "A_ARG_TYPE_Direction" => &["Input", "Output"],
        "A_ARG_TYPE_ConnectionStatus" => &[
            "OK",
            "ContentFormatMismatch",
            "InsufficientBandwidth",
            "UnreliableChannel",
            "Unknown",
        ],
        _ => &[],
    };
    if !values.is_empty() {
        xml.push_str("<allowedValueList>");
        for value in values {
            xml.push_str(&format!("<allowedValue>{value}</allowedValue>"));
        }
        xml.push_str("</allowedValueList>");
    }
    xml.push_str("</stateVariable>");
}
