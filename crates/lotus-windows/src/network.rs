use windows::Networking::Connectivity::NetworkInformation;

const IANA_ETHERNET_CSMACD: u32 = 6;
const IANA_IEEE_802_11: u32 = 71;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NetworkConnectionKind {
    Ethernet,
    Wifi,
    #[default]
    Other,
}

pub fn connection_kind() -> NetworkConnectionKind {
    let Ok(profile) = NetworkInformation::GetInternetConnectionProfile() else {
        return NetworkConnectionKind::Other;
    };
    let Ok(adapter) = profile.NetworkAdapter() else {
        return NetworkConnectionKind::Other;
    };
    let Ok(interface_type) = adapter.IanaInterfaceType() else {
        return NetworkConnectionKind::Other;
    };

    match interface_type {
        IANA_ETHERNET_CSMACD => NetworkConnectionKind::Ethernet,
        IANA_IEEE_802_11 => NetworkConnectionKind::Wifi,
        _ => NetworkConnectionKind::Other,
    }
}
