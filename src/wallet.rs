use anyhow::{anyhow, Chain};
use bdk::{
    bitcoin::{
        secp256k1::{All, Secp256k1},
        Network, Transaction, Txid,
    },
    chain::{
        tx_graph::CanonicalTx, BlockId, ChainPosition, ConfirmationTime, ConfirmationTimeAnchor,
    },
    wallet::wallet_name_from_descriptor,
    Wallet,
};
// use bdk_esplora::{esplora_client, EsploraAsyncExt};
use bdk_file_store::Store;
use bitcoincore_rpc::{
    bitcoincore_rpc_json::{
        ScanBlocksOptions, ScanBlocksRequest, ScanBlocksRequestDescriptor, ScanBlocksResult,
    },
    Auth, Client, RpcApi,
};
use clap::{command, Parser};
use cln_plugin::{Error, Plugin};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, fmt, path::PathBuf, time::Duration};

use crate::state::State;

pub const SMAUG_DATADIR: &str = ".smaug";
const STOP_GAP: usize = 50;
const PARALLEL_REQUESTS: usize = 5;

pub const UTXO_DEPOSIT_TAG: &str = "utxo_deposit";
pub const UTXO_SPENT_TAG: &str = "utxo_spent";

/// Errors related to the `smaug` command.
#[derive(Debug)]
pub enum WatchError {
    InvalidDescriptor(String),
    InvalidChangeDescriptor(String),
    InvalidBirthday(String),
    InvalidGap(String),
    InvalidFormat(String),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct RpcConnection {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
}

impl std::error::Error for WatchError {}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            WatchError::InvalidDescriptor(x) => write!(f, "{x}"),
            WatchError::InvalidChangeDescriptor(x) => write!(f, "{x}"),
            WatchError::InvalidBirthday(x) => write!(f, "{x}"),
            WatchError::InvalidGap(x) => write!(f, "{x}"),
            WatchError::InvalidFormat(x) => write!(f, "{x}"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum WDNetwork {
    #[serde(rename = "bitcoin")]
    Mainnet,
    Testnet,
    Regtest,
    Signet,
    Mutinynet,
}

pub fn get_network_url(network: &str) -> String {
    match network {
        "bitcoin" | "mainnet" => "https://blockstream.info/api".to_owned(),
        "testnet" => "https://blockstream.info/testnet/api".to_owned(),
        "regtest" | "mutinynet" => "https://mutinynet.com/api".to_owned(),
        "signet" => "https://mempool.space/signet/api".to_owned(),
        _ => {
            panic!();
        }
    }
}

pub fn get_network(network: Option<String>) -> Result<Network, Error> {
    return match network {
        Some(n) => match n.as_str() {
            "mutinynet" => Ok(Network::Signet),
            _ => match n.parse::<Network>() {
                Ok(n) => Ok(n),
                Err(e) => return Err(e.into()),
            },
        },
        None => return Err(anyhow!("network is None")),
    };
}

#[derive(Debug, Deserialize, Serialize, Clone, Parser)]
#[command(author, version, about, long_about = None)]
pub struct AddArgs {
    /// External descriptor of wallet to add
    pub descriptor: String,
    /// Internal descriptor of wallet to add
    pub change_descriptor: Option<String>,
    /// Birthday of wallet to add. Must be a block height between 0 and 4294967295
    pub birthday: Option<u32>,
    /// Number of empty addresses to scan before giving up. Must be between 0 and 2147483647
    pub gap: Option<u32>,
}

/// Parameters related to the `smaug` command.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DescriptorWallet {
    pub descriptor: String,
    pub change_descriptor: Option<String>,
    pub birthday: Option<u32>,
    pub gap: Option<u32>,
    // pub last_synced: Option<BlockTime>,
    // #[serde(skip_serializing, skip_deserializing)]
    pub transactions: BTreeMap<Txid, Transaction>,
    pub network: Option<String>,
}
impl DescriptorWallet {
    fn new(
        descriptor: &str,
        change_descriptor: Option<&str>,
        birthday: Option<u64>,
        gap: Option<u64>,
        network: Option<String>,
    ) -> Result<Self, WatchError> {
        let mut params = DescriptorWallet::from_descriptor(descriptor)?;
        if change_descriptor.is_some() {
            params = params.with_change_descriptor(change_descriptor.unwrap())?
        }
        if birthday.is_some() {
            params = params.with_birthday(birthday.unwrap())?
        }
        if gap.is_some() {
            params = params.with_gap(gap.unwrap())?
        }
        if network.is_some() {
            params = params.with_network(network.unwrap())?
        }
        Ok(params)
    }

    pub fn from_args(args: AddArgs, network: String) -> Result<Self, WatchError> {
        Ok(Self {
            descriptor: args.descriptor,
            change_descriptor: args.change_descriptor,
            birthday: args.birthday,
            gap: args.gap,
            transactions: BTreeMap::new(),
            network: Some(network),
        })
    }

    fn from_descriptor(descriptor: &str) -> Result<Self, WatchError> {
        Ok(Self {
            descriptor: descriptor.to_owned(),
            change_descriptor: None,
            birthday: None,
            gap: None,
            // last_synced: None,
            transactions: BTreeMap::new(),
            network: None,
        })
    }

    fn with_change_descriptor(self, change_descriptor: &str) -> Result<Self, WatchError> {
        if change_descriptor.is_empty() {
            Err(WatchError::InvalidChangeDescriptor(
                "change_descriptor is empty".to_owned(),
            ))
        } else {
            Ok(Self {
                change_descriptor: Some(String::from(change_descriptor)),
                ..self
            })
        }
    }

    fn with_birthday(self, birthday: u64) -> Result<Self, WatchError> {
        if birthday > u32::MAX as u64 {
            Err(WatchError::InvalidBirthday(format!(
                "birthday must be between 0 and 4294967295. Received: {birthday}"
            )))
        } else {
            Ok(Self {
                birthday: Some(birthday as u32),
                ..self
            })
        }
    }

    fn with_gap(self, gap: u64) -> Result<Self, WatchError> {
        if gap > u32::MAX as u64 / 2 {
            Err(WatchError::InvalidBirthday(format!(
                "gap must be between 0 and 2147483647. Received: {gap}"
            )))
        } else {
            Ok(Self {
                gap: Some(gap as u32),
                ..self
            })
        }
    }

    fn with_network(self, network: String) -> Result<Self, WatchError> {
        Ok(Self {
            network: Some(network),
            ..self
        })
    }

    // pub fn update_last_synced(&mut self, last_synced: BlockTime) {
    //     self.last_synced = Some(last_synced);
    // }

    pub fn update_transactions<'a>(
        &mut self,
        transactions: Vec<CanonicalTx<'a, Transaction, ConfirmationTimeAnchor>>,
    ) -> Vec<CanonicalTx<'a, Transaction, ConfirmationTimeAnchor>> {
        let mut new_txs = vec![];
        for tx in transactions {
            if !self.transactions.contains_key(&tx.tx_node.txid) {
                new_txs.push(tx.clone());
                self.transactions
                    .insert(tx.tx_node.txid.clone(), tx.tx_node.tx.clone());
            }
        }
        new_txs
        // self.transactions = transactions;
    }

    pub fn get_network(&self) -> Result<Network, Error> {
        get_network(self.network.clone())
    }

    pub fn get_name(&self) -> Result<String, Error> {
        log::info!("get_name called");
        let network = get_network(self.network.clone());
        log::info!("get_network succeeded");
        Ok(wallet_name_from_descriptor(
            &self.descriptor,
            self.change_descriptor.as_ref(),
            network?,
            &Secp256k1::<All>::new(),
        )?)
    }

    pub async fn fetch_wallet<'a>(
        &self,
        db_dir: PathBuf,
        brpc_host: String,
        brpc_port: u16,
        brpc_user: String,
        brpc_pass: String,
        // ) -> Result<Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>, Error>
    ) -> Result<Wallet<Store<'_, bdk::wallet::ChangeSet>>, Error> {
        log::trace!("creating path");
        let db_filename = self.get_name()?;
        let db_path = db_dir
            // .join(DATADIR)
            .join(format!("{}.db", db_filename,));
        log::trace!("searching for path: {:?}", db_path);
        // let db = Store::<bdk::wallet::ChangeSet>::new_from_path(SMAUG_DATADIR.as_bytes(), db_path)?;
        let db = Store::<bdk::wallet::ChangeSet>::new_from_path(SMAUG_DATADIR.as_bytes(), db_path)?;
        log::trace!("db created!");
        // let external_descriptor = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/0'/0'/0/*)";
        // mutinynet_descriptor = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/*)"
        let _mutinynet_descriptor_ext = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/84'/0'/0'/0/*)";
        let _mutinynet_descriptor_int = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/84'/0'/0'/1/*)";
        let _mutinynet_descriptor_ext_2 = "wpkh(tprv8ZgxMBicQKsPeRye8MhHA8hLxMuomycmGYXyRs7zViNck2VJsCJMTPt81Que8qp3PyPgQRnN7Gb1JyBVBKgj8AKEoEmmYxYDwzZJ63q1yjA/84'/0'/0'/0/*)";
        let _mutinynet_descriptor_int_2 = "wpkh(tprv8ZgxMBicQKsPeRye8MhHA8hLxMuomycmGYXyRs7zViNck2VJsCJMTPt81Que8qp3PyPgQRnN7Gb1JyBVBKgj8AKEoEmmYxYDwzZJ63q1yjA/84'/0'/0'/1/*)";
        // let external_descriptor = "wpkh(tpubEBr4i6yk5nf5DAaJpsi9N2pPYBeJ7fZ5Z9rmN4977iYLCGco1VyjB9tvvuvYtfZzjD5A8igzgw3HeWeeKFmanHYqksqZXYXGsw5zjnj7KM9/*)";
        // let internal_descriptor = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/0'/0'/1/*)";

        // let external_descriptor = mutinynet_descriptor_ext;
        // let internal_descriptor = mutinynet_descriptor_int;
        let external_descriptor = self.descriptor.clone();
        let internal_descriptor = self.change_descriptor.clone();
        let mut wallet = Wallet::new(
            &external_descriptor,
            internal_descriptor.as_ref(),
            db,
            self.get_network()?,
        )?;
        log::trace!("wallet created!");

        let balance = wallet.get_balance();
        log::trace!("Wallet balance before syncing: {} sats", balance.total());

        log::trace!("Syncing...");
        log::debug!("using network: {}", json!(self.network).as_str().unwrap());
        // log::debug!(
        //     "using esplora url: {}",
        //     get_network_url(json!(self.network).as_str().unwrap()).as_str()
        // );
        // let client =
        //     // esplora_client::Builder::new("https://blockstream.info/testnet/api").build_async()?;
        //     esplora_client::Builder::new(
        //         get_network_url(
        //                 json!(self.network).as_str().unwrap()
        //         ).as_str()
        //     ).build_async()?;
        // let client =
        //     // esplora_client::Builder::new("https://blockstream.info/testnet/api").build_async()?;
        //     esplora_client::Builder::new(
        //         get_network_url(
        //                 json!(self.network).as_str().unwrap()
        //         ).as_str()
        //     ).build_async()?;

        // let local_chain = wallet.checkpoints();
        // let keychain_spks = wallet
        //     .spks_of_all_keychains()
        //     .into_iter()
        //     .map(|(k, k_spks)| {
        //         let mut once = Some(());
        //         let mut stdout = std::io::stdout();
        //         let k_spks = k_spks
        //             .inspect(move |(spk_i, _)| match once.take() {
        //                 Some(_) => log::debug!("\nScanning keychain [{:?}]", k),
        //                 None => log::trace!(" {:<3}", spk_i),
        //             })
        //             .inspect(move |_| stdout.flush().expect("must flush"));
        //         (k, k_spks)
        //     })
        //     .collect();
        // log::trace!("Finished scanning");
        // let update = client
        //     .scan(
        //         local_chain,
        //         keychain_spks,
        //         [],
        //         [],
        //         STOP_GAP,
        //         PARALLEL_REQUESTS,
        //     )
        //     .await?;
        // wallet.apply_update(update)?;
        // wallet.commit()?;

        let rpc_client = Client::new_with_timeout(
            &format!("http://{}:{}", brpc_host.clone(), brpc_port.clone()),
            Auth::UserPass(brpc_user.clone(), brpc_pass.clone()), // Auth::CookieFile(PathBuf::from("/home/cguida/.bitcoin/regtest/.cookie"))
            Duration::from_secs(3600),
        )?;

        println!(
            "Connected to Bitcoin Core RPC at {:?}",
            rpc_client.get_blockchain_info().unwrap()
        );

        let external_descriptor = ScanBlocksRequestDescriptor::Extended {
            desc: external_descriptor.to_string(),
            range: None,
        };
        let mut descriptors_vec = vec![external_descriptor];

        if let Some(id) = internal_descriptor {
            descriptors_vec.push(ScanBlocksRequestDescriptor::Extended {
                desc: id.to_string(),
                range: None,
            });
        }

        // let descriptors = &[external_descriptor, internal_descriptor];
        let descriptors = &descriptors_vec[..];
        let request = ScanBlocksRequest {
            scanobjects: descriptors,
            start_height: None,
            stop_height: None,
            filtertype: None,
            options: Some(ScanBlocksOptions {
                filter_false_positives: Some(true),
            }),
        };
        let res: ScanBlocksResult = rpc_client.scan_blocks_blocking(request)?;
        // let res: ScanBlocksResult = ScanBlocksResult {
        //     from_height: 0,
        //     to_height: 819333,
        //     relevant_blocks: vec![
        //         BlockHash::from_str(
        //             "000000000000000000047a0baacb20399819c82d6983a545d849625c040380e5",
        //         )?,
        //         BlockHash::from_str(
        //             "0000000000000000000114f60040b10b192bc37d3f1f5777686509898106105e",
        //         )?,
        //         BlockHash::from_str(
        //             "000000000000000000031359d3aff6ecfb95995bc9b84b079302836db45174ed",
        //         )?,
        //     ],
        // };
        println!("scanblocks result: {:?}", res);
        println!("wallet = {:?}", wallet);

        wallet.set_lookahead_for_all(20)?;

        // let chain_tip = wallet.latest_checkpoint();
        // let mut emitter = match chain_tip {
        //     Some(cp) => Emitter::from_checkpoint(&rpc_client, cp),
        //     None => Emitter::from_height(&rpc_client, args[5].parse::<u32>()?),
        // };

        let mut prev_block_id = None;

        for bh in res.relevant_blocks {
            // self.get_relevant_txs(bh, &conn);
            let block = rpc_client.get_block(&bh)?;
            let height: u32 = block.bip34_block_height()?.try_into().unwrap();
            println!("adding block height {} to wallet", height);
            wallet.apply_block_relevant(block.clone(), prev_block_id, height)?;
            wallet.commit()?;
            prev_block_id = Some(BlockId { height, hash: bh });
        }

        // while let Some((height, block)) = emitter.next_block()? {
        //     println!("Applying block {} at height {}", block.block_hash(), height);
        //     wallet.apply_block_relevant(block, height)?;
        //     wallet.commit()?;
        // }

        // println!("About to apply unconfirmed transactions: ...");
        // let unconfirmed_txs = emitter.mempool()?;
        // println!("Applying unconfirmed transactions: ...");
        // wallet.batch_insert_relevant_unconfirmed(unconfirmed_txs.iter().map(|(tx, time)| (tx, *time)));
        // wallet.commit()?;

        let balance = wallet.get_balance();
        println!("Wallet balance after syncing: {} sats", balance.total());

        let balance = wallet.get_balance();
        log::trace!("Wallet balance after syncing: {} sats", balance.total());
        return Ok(wallet);
    }

    // pub async fn scanblocks<'a>(
    //     &self,
    //     brpc_host: String,
    //     brpc_port: u16,
    //     brpc_user: String,
    //     brpc_pass: String,
    // ) -> Result<(), Error> {
    //     // let external_descriptor = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/0'/0'/0/*)";
    //     // mutinynet_descriptor = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/*)"
    //     let _mutinynet_descriptor_ext = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/84'/0'/0'/0/*)";
    //     let _mutinynet_descriptor_int = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/84'/0'/0'/1/*)";
    //     let _mutinynet_descriptor_ext_2 = "wpkh(tprv8ZgxMBicQKsPeRye8MhHA8hLxMuomycmGYXyRs7zViNck2VJsCJMTPt81Que8qp3PyPgQRnN7Gb1JyBVBKgj8AKEoEmmYxYDwzZJ63q1yjA/84'/0'/0'/0/*)";
    //     let _mutinynet_descriptor_int_2 = "wpkh(tprv8ZgxMBicQKsPeRye8MhHA8hLxMuomycmGYXyRs7zViNck2VJsCJMTPt81Que8qp3PyPgQRnN7Gb1JyBVBKgj8AKEoEmmYxYDwzZJ63q1yjA/84'/0'/0'/1/*)";

    //     let rpc = Client::new_with_timeout(
    //         &format!("http://{}:{}", brpc_host, brpc_port),
    //         Auth::UserPass(brpc_user.clone(), brpc_pass.clone()), // Auth::CookieFile(PathBuf::from("/home/cguida/.bitcoin/regtest/.cookie"))
    //         Duration::from_secs(3600),
    //     )?;
    //     let descriptor = ScanBlocksRequestDescriptor::Extended {
    //         desc: self.descriptor.clone().to_string(),
    //         range: None,
    //     };
    //     let descriptors = &[descriptor];
    //     let request = ScanBlocksRequest {
    //         scanobjects: descriptors,
    //         start_height: None,
    //         stop_height: None,
    //         filtertype: None,
    //         options: Some(ScanBlocksOptions {
    //             filter_false_positives: Some(true),
    //         }),
    //     };
    //     let res = rpc.scan_blocks_blocking(request)?;
    //     log::info!("scanblocks result: {:?}", res);

    //     let conn = RpcConnection {
    //         host: brpc_host,
    //         port: brpc_port,
    //         user: brpc_user,
    //         pass: brpc_pass,
    //     };

    //     for bh in res.relevant_blocks {
    //         self.get_relevant_txs(bh, &conn).await?;
    //     }

    //     return Ok(());
    // }

    // async fn get_relevant_txs(
    //     &self,
    //     bh: BlockHash,
    //     conn: &RpcConnection,
    // ) -> Result<Vec<Transaction>, Error> {
    //     let mut relevant_txs: Vec<Transaction> = vec![];
    //     let rpc = Client::new(
    //         &format!("http://{}:{}", conn.host, conn.port),
    //         Auth::UserPass(conn.user.clone(), conn.pass.clone()), // Auth::CookieFile(PathBuf::from("/home/cguida/.bitcoin/regtest/.cookie"))
    //                                                               // Duration::from_secs(3600),
    //     )?;
    //     let block = rpc.get_block(&bh)?;
    //     for tx in block.txdata {
    //         // let tx_bdk = tx.into();
    //         let chain_update =
    //         CheckPoint::from_header(&block.header, height).into_update(false);
    //         let chain_changeset = chain
    //             .apply_update(chain_update)
    //             .expect("must always apply as we receive blocks in order from emitter");
    //         let graph_changeset = graph.apply_block_relevant(block, height);
    //         (chain_changeset, graph_changeset)
    //             relevant_txs.push(tx);
    //     }
    //     Ok(relevant_txs)
    // }

    // assume we own all inputs, ie sent from our wallet. all inputs and outputs should generate coin movement bookkeeper events
    async fn spend_tx_notify<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'_, bdk::wallet::ChangeSet>>,
        tx: &CanonicalTx<'_, Transaction, ConfirmationTimeAnchor>,
    ) -> Result<(), Error> {
        // match tx {
        // Some(t) => {
        // send spent notification for each input
        for input in tx.tx_node.tx.input.iter() {
            if let Some(po) = wallet.tx_graph().get_txout(input.previous_output) {
                match tx.chain_position {
                    //     ChainPosition::Confirmed(a) => Self::Confirmed {
                    //         height: a.confirmation_height,
                    //         time: a.confirmation_time,
                    //     },
                    //     ChainPosition::Unconfirmed(_) => Self::Unconfirmed { last_seen: 0 },
                    // }
                    // match ConfirmationTime::from(&) {
                    // ConfirmationTime::Unconfirmed { .. } => {
                    ChainPosition::Unconfirmed(_) => {
                        continue;
                    }
                    ChainPosition::Confirmed(a) => {
                        let acct = format!("smaug:{}", self.get_name()?);
                        let amount = po.value;
                        let outpoint = format!("{}", input.previous_output.to_string());
                        log::trace!("outpoint = {}", format!("{}", outpoint));
                        let onchain_spend = json!({UTXO_SPENT_TAG: {
                            "account": acct,
                            "outpoint": outpoint,
                            "spending_txid": tx.tx_node.txid,
                            "amount_msat": amount,
                            "coin_type": "bcrt",
                            "timestamp": format!("{}", a.confirmation_time),
                            "blockheight": format!("{}", a.confirmation_height),
                        }});
                        log::trace!("INSIDE SEND SPEND NOTIFICATION ON SMAUG SIDE");
                        let cloned_plugin = plugin.clone();
                        tokio::spawn(async move {
                            if let Err(e) = cloned_plugin
                                .send_custom_notification(UTXO_SPENT_TAG.to_string(), onchain_spend)
                                .await
                            {
                                log::error!("Error sending custom notification: {:?}", e);
                            }
                        });
                    }
                }
            } else {
                log::trace!("Transaction prevout not found");
            }
        }

        // send deposit notification for every output, since all of them are spends from our wallet
        for (vout, output) in tx.tx_node.tx.output.iter().enumerate() {
            match tx.chain_position {
                ChainPosition::Unconfirmed(_) => {
                    continue;
                }
                ChainPosition::Confirmed(a) => {
                    let acct: String;
                    let transfer_from: String;
                    if wallet.is_mine(&output.script_pubkey) {
                        acct = format!("smaug:{}", self.get_name()?);
                        transfer_from = "external".to_owned();
                    } else {
                        transfer_from = format!("smaug:{}", self.get_name()?);
                        acct = "external".to_owned();
                    }
                    let amount = output.value;
                    let outpoint = format!("{}:{}", tx.tx_node.txid.to_string(), vout.to_string());
                    log::trace!("outpoint = {}", format!("{}:{}", tx.tx_node.txid, vout));
                    let onchain_deposit = json!({UTXO_DEPOSIT_TAG:{
                            "account": acct,
                            "transfer_from": transfer_from,
                            "outpoint": outpoint,
                            "spending_txid": tx.tx_node.txid,
                            "amount_msat": amount,
                            "coin_type": "bcrt",
                            "timestamp": format!("{}", a.confirmation_time),
                            "blockheight": format!("{}", a.confirmation_height),
                    }});
                    log::trace!("INSIDE SEND DEPOSIT NOTIFICATION ON SMAUG SIDE");
                    let cloned_plugin = plugin.clone();
                    tokio::spawn(async move {
                        if let Err(e) = cloned_plugin
                            .send_custom_notification(UTXO_DEPOSIT_TAG.to_string(), onchain_deposit)
                            .await
                        {
                            log::error!("Error sending custom notification: {:?}", e);
                        }
                    });
                }
            }
        }
        Ok(())
        // }
        // None => {
        //     log::debug!("Transaction is missing a Transaction");
        // }
        // }
    }

    // assume we own no inputs. sent to us from someone else's wallet.
    // all outputs we own should generate utxo deposit events.
    // outputs we don't own should not generate events.
    async fn receive_tx_notify<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'_, bdk::wallet::ChangeSet>>,
        tx: &CanonicalTx<'_, Transaction, ConfirmationTimeAnchor>,
    ) -> Result<(), Error> {
        // match tx.transaction.clone() {
        //     Some(t) => {
        for (vout, output) in tx.tx_node.tx.output.iter().enumerate() {
            if wallet.is_mine(&output.script_pubkey) {
                match tx.chain_position {
                    ChainPosition::Unconfirmed(_) => {
                        continue;
                    }
                    ChainPosition::Confirmed(a) => {
                        let acct: String;
                        let transfer_from: String;
                        if wallet.is_mine(&output.script_pubkey) {
                            acct = format!("smaug:{}", self.get_name()?);
                            transfer_from = "external".to_owned();
                        } else {
                            // transfer_from = format!(
                            //     "smaug:{}",
                            //     self.get_name?
                            // );
                            // acct = "external".to_owned();
                            continue;
                        }
                        let amount = output.value;
                        let outpoint = format!("{}:{}", tx.tx_node.txid, vout);
                        log::trace!(
                            "outpoint = {}",
                            format!("{}:{}", tx.tx_node.txid.to_string(), vout.to_string())
                        );
                        let onchain_deposit = json!({UTXO_DEPOSIT_TAG: {
                                "account": acct,
                                "transfer_from": transfer_from,
                                "outpoint": outpoint,
                                "spending_txid": tx.tx_node.txid.to_string(),
                                "amount_msat": amount,
                                "coin_type": "bcrt",
                                "timestamp": format!("{}", a.confirmation_time),
                                "blockheight": format!("{}", a.confirmation_height),
                        }});
                        log::trace!("INSIDE SEND DEPOSIT NOTIFICATION ON SMAUG SIDE");
                        let cloned_plugin = plugin.clone();
                        tokio::spawn(async move {
                            if let Err(e) = cloned_plugin
                                .send_custom_notification(
                                    UTXO_DEPOSIT_TAG.to_string(),
                                    onchain_deposit,
                                )
                                .await
                            {
                                log::error!("Error sending custom notification: {:?}", e);
                            }
                        });
                    }
                }
            }
        }
        //     }
        //     None => {
        //         log::debug!("Transaction is missing a Transaction");
        //     }
        // }
        Ok(())
    }

    // assume we own some inputs and not others.
    // this tx was generated collaboratively between our wallet and (an)other wallet(s).
    // send events for all our owned inputs.
    // request manual intervention to identify which outputs are ours. send them to bkpr in a temporary account?
    async fn shared_tx_notify<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'_, bdk::wallet::ChangeSet>>,
        tx: &CanonicalTx<'_, Transaction, ConfirmationTimeAnchor>,
    ) -> Result<(), Error> {
        // match tx.transaction.clone() {
        //     Some(t) => {
        // send spent notification for each input that spends one of our outputs
        for input in tx.tx_node.input.iter() {
            if let Some(po) = wallet.tx_graph().get_txout(input.previous_output) {
                match tx.chain_position {
                    ChainPosition::Unconfirmed(_) => {
                        continue;
                    }
                    ChainPosition::Confirmed(a) => {
                        if wallet.is_mine(&po.script_pubkey) {
                            let acct = format!("smaug:{}", self.get_name()?);
                            let amount = po.value;
                            let outpoint = format!("{}", input.previous_output.to_string());
                            log::trace!("outpoint = {}", format!("{}", outpoint));
                            let onchain_spend = json!({UTXO_SPENT_TAG: {
                                "account": acct,
                                "outpoint": outpoint,
                                "spending_txid": tx.tx_node.txid.to_string(),
                                "amount_msat": amount,
                                "coin_type": "bcrt",
                                "timestamp": format!("{}", a.confirmation_time),
                                "blockheight": format!("{}", a.confirmation_height),
                            }});
                            log::trace!("INSIDE SEND SPEND NOTIFICATION ON SMAUG SIDE");
                            let cloned_plugin = plugin.clone();
                            tokio::spawn(async move {
                                if let Err(e) = cloned_plugin
                                    .send_custom_notification(
                                        UTXO_SPENT_TAG.to_string(),
                                        onchain_spend,
                                    )
                                    .await
                                {
                                    log::error!("Error sending custom notification: {:?}", e);
                                }
                            });
                        }
                    }
                }
            } else {
                log::debug!("Transaction prevout not found");
            }
        }

        // send deposit notification for every output, since all of them *might be* spends from our wallet.
        // store them in a temp account and let the user update later as needed.
        for (vout, output) in tx.tx_node.tx.output.iter().enumerate() {
            match tx.chain_position {
                ChainPosition::Unconfirmed(_) => {
                    continue;
                }
                ChainPosition::Confirmed(a) => {
                    let acct: String;
                    let transfer_from: String;
                    let our_acct = format!("smaug:{}:shared_outputs", self.get_name()?);
                    let ext_acct = "external".to_owned();
                    if wallet.is_mine(&output.script_pubkey) {
                        acct = our_acct;
                        transfer_from = ext_acct;
                    } else {
                        acct = ext_acct;
                        transfer_from = our_acct;
                    }
                    let amount = output.value;
                    let outpoint = format!("{}:{}", tx.tx_node.txid, vout);
                    log::trace!("outpoint = {}", format!("{}:{}", tx.tx_node.txid, vout));
                    let onchain_deposit = json!({UTXO_DEPOSIT_TAG: {
                            "account": acct,
                            "transfer_from": transfer_from,
                            "outpoint": outpoint,
                            "spending_txid": tx.tx_node.txid,
                            "amount_msat": amount,
                            "coin_type": "bcrt",
                            "timestamp": format!("{}", a.confirmation_time),
                            "blockheight": format!("{}", a.confirmation_height),
                    }});
                    log::trace!("INSIDE SEND DEPOSIT NOTIFICATION ON SMAUG SIDE");
                    let cloned_plugin = plugin.clone();
                    tokio::spawn(async move {
                        if let Err(e) = cloned_plugin
                            .send_custom_notification(UTXO_DEPOSIT_TAG.to_string(), onchain_deposit)
                            .await
                        {
                            log::error!("Error sending custom notification: {:?}", e);
                        }
                    });
                }
            }
        }
        //     }
        //     None => {
        //         log::debug!("Transaction is missing a Transaction");
        //     }
        // }
        Ok(())
    }

    pub async fn send_notifications_for_tx<'a>(
        &self,
        plugin: &Plugin<State>,
        wallet: &Wallet<Store<'_, bdk::wallet::ChangeSet>>,
        tx: CanonicalTx<'_, Transaction, ConfirmationTimeAnchor>,
    ) -> Result<(), Error> {
        log::debug!("sending notifs for txid/tx: {:?} {:?}", tx.tx_node.txid, tx);
        // we own all inputs
        if tx.clone().tx_node.tx.input.iter().all(|x| {
            match wallet.tx_graph().get_txout(x.previous_output) {
                Some(o) => {
                    log::trace!(
                        "output is mine?: {:?} {:?}",
                        o,
                        wallet.is_mine(&o.script_pubkey)
                    );
                    wallet.is_mine(&o.script_pubkey)
                }
                None => {
                    log::trace!("output not found in tx graph: {:?}", x.previous_output);
                    false
                }
            }
        }) {
            log::debug!("sending spend notif");
            self.spend_tx_notify(plugin, wallet, &tx).await?;
        } else
        // we own no inputs
        if !tx.clone().tx_node.tx.input.iter().any(|x| {
            match wallet.tx_graph().get_txout(x.previous_output) {
                Some(o) => {
                    log::trace!(
                        "output is mine?: {:?} {:?}",
                        o,
                        wallet.is_mine(&o.script_pubkey)
                    );
                    wallet.is_mine(&o.script_pubkey)
                }
                None => {
                    log::trace!("output not found in tx graph: {:?}", x.previous_output);
                    false
                }
            }
        }) {
            log::debug!("sending deposit notif");
            self.receive_tx_notify(plugin, wallet, &tx).await?;
        }
        // we own some inputs but not others
        else {
            log::debug!("sending shared notif");
            self.shared_tx_notify(plugin, wallet, &tx).await?;
        }

        // if tx.sent > 0 {

        // }

        // if tx.received > 0 {

        // }
        Ok(())
    }
}

impl TryFrom<serde_json::Value> for DescriptorWallet {
    type Error = WatchError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        log::trace!("entering try_from");
        match value {
            serde_json::Value::Array(a) => {
                log::trace!("try_from: array detected = {:?}", a);
                let param_count = a.len();

                match param_count {
                    // 1 => DescriptorWallet::try_from(a.pop().unwrap()),
                    1..=4 => {
                        let descriptor = a.get(0).unwrap().as_str().ok_or_else(|| WatchError::InvalidDescriptor("descriptor must be a string".to_string()))?;
                        // let change_descriptor = Some(a.get(1).unwrap().as_str().ok_or_else(|| WatchError::InvalidChangeDescriptor("change_descriptor must be a string".to_string()))?);
                        log::trace!("try_from array: change_descriptor = {:?}", a.get(1));
                        let change_descriptor = if let Some(cd) = a.get(1) {
                            Some(cd.as_str().ok_or_else(|| WatchError::InvalidChangeDescriptor(format!("change_descriptor must be a string. Received: {cd}")))?)
                        } else {
                            None
                        };
                        let birthday = if let Some(b) = a.get(2) {
                            Some(b.as_u64().ok_or_else(|| WatchError::InvalidBirthday(format!("birthday must be a number. Received: {b}")))?)
                        } else {
                            None
                        };
                        let gap = if let Some(g) = a.get(3) {
                            Some(g.as_u64().ok_or_else(|| WatchError::InvalidGap(format!("gap must be a number. Received: {g}")))?)
                        } else {
                            None
                        };

                        DescriptorWallet::new(descriptor, change_descriptor, birthday, gap, None)
                    }
                    _ => Err(WatchError::InvalidFormat(format!("Unexpected request format. The request needs 1-4 parameters. Received: {param_count}"))),
                }
            },
            serde_json::Value::Object(m) => {
                log::trace!("try_from: object detected");
                let allowed_keys = ["descriptor", "change_descriptor", "birthday", "gap"];
                let param_count = m.len();

                 if m.is_empty() || param_count > allowed_keys.len() {
                    Err(WatchError::InvalidFormat(format!("Unexpected request format. The request needs 1-4 parameters. Received: {param_count}")))
                 } else if !m.contains_key(allowed_keys[0]){
                    Err(WatchError::InvalidDescriptor(format!("{} is mandatory", allowed_keys[0])))
                 } else if !m.iter().all(|(k, _)| allowed_keys.contains(&k.as_str())) {
                    Err(WatchError::InvalidFormat(format!("Invalid named parameter found in request. Allowed named params: ['descriptor', 'change_descriptor', 'birthday', 'gap']")))
                 } else {
                    DescriptorWallet::new(
                        m.get("descriptor").unwrap().as_str().unwrap(),
                        match m.get("change_descriptor") {
                            Some(v) => Some(v.as_str().unwrap()),
                            None => None,
                        },
                        match m.get("birthday") {
                            Some(v) => Some(v.as_u64().unwrap()),
                            None => None,
                        },
                        match m.get("gap") {
                            Some(v) => Some(v.as_u64().unwrap()),
                            None => None,
                        },
                        None,
                    )
                }
            },
            _ => Err(WatchError::InvalidFormat(
                format!("Unexpected request format. Expected: <descriptor>, [change_descriptor, birthday, gap], either as ordered or keyword args. Received: '{value}'"),
            )),
        }
    }
}
