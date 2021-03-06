use clarity::Address;
use num256::Uint256;

fn default_node_list() -> Vec<String> {
    vec![
        "https://eth.althea.org:443".to_string(),
        "https://mainnet.infura.io/v3/6b080f02d7004a8394444cdf232a7081".to_string(),
    ]
}

fn default_dao_address() -> Vec<Address> {
    Vec::new()
}

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq, Default)]
pub struct SubnetDAOSettings {
    /// A list of nodes to query for blockchain data
    /// this is kept seperate from the version for payment settings node
    /// list in order to allow for the DAO and payments to exist on different
    /// chains, provided in name:port format
    #[serde(default = "default_node_list")]
    pub node_list: Vec<String>,
    /// List of subnet DAO's to which we are a member
    #[serde(default = "default_dao_address")]
    pub dao_addresses: Vec<Address>,
    /// The amount in wei that will be sent to the dao in one second
    #[serde(default)]
    pub dao_fee: Uint256,
}
