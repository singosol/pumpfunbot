use solana_client::rpc_client::{GetConfirmedSignaturesForAddress2Config, RpcClient};
use solana_client::rpc_config::RpcTransactionConfig;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{read_keypair_file, Keypair, Signature, Signer};
use solana_sdk::transaction::Transaction;
use solana_transaction_status::option_serializer::OptionSerializer;
use solana_transaction_status::UiTransactionTokenBalance;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use tokio::time::{self, sleep, Duration, Instant};
use solana_account_decoder::{UiAccountData};
use serde::{Deserialize, Serialize};
use serde_json;
use std::io::Cursor;
use std::error::Error;
use std::sync::{Arc, Mutex};
use reqwest;
use reqwest::Client;
use serde_json::json;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_sdk::instruction::{AccountMeta, Instruction};
use solana_sdk::compute_budget::{ComputeBudgetInstruction};
use solana_sdk::system_program::ID as SYSTEM_PROGRAM_ID;
use byteorder::{LittleEndian, WriteBytesExt, ReadBytesExt};


const RPC_URL: &str = "your_rpc_url";//替换成你的rpc链接，使用例如helius等私有rpc链接，不要使用公共rcp链接
const DISCORD_WEBHOOK_URL: &str = "your_discord_webhook_url";//替换成你的discord_webhook_url链接
const KEYPAIR: &str = "";//替换成你自己的密钥路径，在生成solana地址时会输出这个路径,例如/home/ls/.config/solana/id.json
const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
const UNIT_PRICE: u64 = 2_000_000;
const UNIT_BUDGET: u64 = 100_000;
const PUMP_FUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JupiterQuote {
    input_mint: String,
    in_amount: String,
    output_mint: String,
    out_amount: String,
    other_amount_threshold: String,
    swap_mode: String,
    slippage_bps: u64,
    platform_fee: Option<PlatformFee>,
    price_impact_pct: String,
    route_plan: Vec<RoutePlan>,
    context_slot: u64,
    time_taken: f64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlatformFee {
    amount: String,
    fee_bps: u64,
    fee_account: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoutePlan {
    swap_info: SwapInfo,
    percent: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SwapInfo {
    amm_key: String,
    label: String,
    input_mint: String,
    output_mint: String,
    in_amount: String,
    out_amount: String,
    fee_amount: String,
    fee_mint: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SwapResponse {
    #[serde(rename = "swapTransaction")]
    swap_transaction: String,
}

#[derive(Clone)]
struct TokenMonitoringInfo {
    #[allow(dead_code)]
    mint_address: String,
    start_time: Instant,
    purchase_price: Option<f64>,
    highest_price: f64,
    price_history: Vec<f64>,
    is_monitoring_phase: bool,
    buy_time: Option<Instant>,
    name: Option<String>,
    symbol: Option<String>,
}

#[derive(Debug, BorshDeserialize, BorshSerialize)]
struct Metadata {
    pub key: u8,
    pub update_authority: Pubkey,
    pub mint: Pubkey,
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub seller_fee_basis_points: u16,
    pub creators: Option<Vec<Creator>>,
}

#[derive(Debug, BorshDeserialize, BorshSerialize)]
struct Creator {
    pub address: Pubkey,
    pub verified: bool,
    pub share: u8,
}

struct VirtualReserves {
    virtual_token_reserves: u64,
    virtual_sol_reserves: u64,
    //token_total_supply: u64,
    //complete: bool,
    bonding_curve: Pubkey,
    associated_bonding_curve: Pubkey,
}

fn derive_bonding_curve_accounts(mint_str: &str) -> Result<(Pubkey, Pubkey), Box<dyn Error>> {
    let mint = Pubkey::from_str(mint_str)?;

    let pump_fun_program = Pubkey::from_str(PUMP_FUN_PROGRAM)?;

    let seeds: &[&[u8]] = &[b"bonding-curve", mint.as_ref()];
    let (bonding_curve, _) = Pubkey::find_program_address(seeds, &pump_fun_program);

    let associated_bonding_curve = spl_associated_token_account::get_associated_token_address(&bonding_curve, &mint);
    
    Ok((bonding_curve, associated_bonding_curve))
}

fn get_virtual_reserves(client: &RpcClient, bonding_curve: &Pubkey) -> Result<VirtualReserves, Box<dyn Error>> {
    let account_info = client.get_account_data(bonding_curve)?;
    let mut data = Cursor::new(&account_info);

    data.set_position(8);
    let virtual_token_reserves = data.read_u64::<LittleEndian>()?;
    let virtual_sol_reserves = data.read_u64::<LittleEndian>()?;
    //let token_total_supply = data.read_u64::<LittleEndian>()?;
    //let complete = data.read_u8()? != 0;

    Ok(VirtualReserves {
        virtual_token_reserves,
        virtual_sol_reserves,
        //token_total_supply,
        //complete,
        bonding_curve: *bonding_curve,
        associated_bonding_curve: spl_associated_token_account::get_associated_token_address(bonding_curve, &Pubkey::from_str(PUMP_FUN_PROGRAM)?),
    })
}

fn get_coin_data(client: &RpcClient, mint_str: &str) -> Result<VirtualReserves, Box<dyn Error>> {
    let (bonding_curve, associated_bonding_curve) = derive_bonding_curve_accounts(mint_str)?;
    let mut reserves = get_virtual_reserves(client, &bonding_curve)?;
    reserves.bonding_curve = bonding_curve;
    reserves.associated_bonding_curve = associated_bonding_curve;
    Ok(reserves)
}

fn get_token_price(client: &RpcClient, mint_str: &str) -> Result<f64, Box<dyn Error>> {
    let coin_data = get_coin_data(client, mint_str)?;
    let virtual_sol_reserves = coin_data.virtual_sol_reserves as f64 / 1_000_000_000_f64;
    let virtual_token_reserves = coin_data.virtual_token_reserves as f64 / 1_000_000_f64;
    let token_price = virtual_sol_reserves / virtual_token_reserves;
    let formatted_price = format!("{:.12}", token_price);
    let parsed_price: f64 = formatted_price.parse()?;
    Ok(parsed_price)
}

async fn get_token_prices(
    client: Arc<RpcClient>,
    mint_addresses: &[String],
    sol_price: Arc<Mutex<f64>>,
) -> Result<HashMap<String, f64>, Box<dyn Error + Send + Sync>> {
    let mut tasks = vec![];

    for mint in mint_addresses {
        let client_clone = Arc::clone(&client);
        let sol_price_clone = Arc::clone(&sol_price);
        let mint_clone = mint.clone();
        
        let task = tokio::spawn(async move {
            if let Ok(token_price_in_sol) = get_token_price(&client_clone, &mint_clone) {
                let sol_price_guard = sol_price_clone.lock().unwrap();
                let sol_price_in_usdc = *sol_price_guard;

                let token_price_in_usdc = token_price_in_sol * sol_price_in_usdc;
                Ok::<(String, f64), Box<dyn Error + Send + Sync>>((mint_clone, token_price_in_usdc))
            } else {
                Err("无法获取代币价格".into())
            }
        });

        tasks.push(task);
    }

    let results = futures::future::join_all(tasks).await;

    let mut prices = HashMap::new();
    for result in results {
        if let Ok(Ok((mint, price))) = result {
            //println!("代币 {} 价格 {}", mint, price);
            prices.insert(mint, price);
        }
    }

    Ok(prices)
}

async fn update_sol_price_in_usdc(sol_price: Arc<Mutex<f64>>) -> Result<(), Box<dyn Error>> {
    let mut interval = time::interval(Duration::from_secs(60)); 
    loop {
        interval.tick().await;
        match get_sol_price_in_usdc().await {
            Ok(price) => {
                let mut sol_price_guard = sol_price.lock().unwrap();
                *sol_price_guard = price;
                println!("更新SOL价格: {}", price);
            }
            Err(e) => {
                println!("更新SOL错误: {}", e);
            }
        }
    }
}

async fn get_sol_price_in_usdc() -> Result<f64, Box<dyn Error>> {
    let url = "https://api.jup.ag/price/v2?ids=So11111111111111111111111111111111111111112";
    let response: serde_json::Value = reqwest::get(url).await?.json().await?;

    let sol_price_str = response["data"]["So11111111111111111111111111111111111111112"]["price"]
        .as_str()
        .ok_or("Failed to find SOL price as a string")?;
    
    let sol_price_in_usdc: f64 = sol_price_str.parse()?; 

    Ok(sol_price_in_usdc)
}

async fn get_top_holders(
    client: Arc<RpcClient>, 
    mint_address: &str
) -> Result<(bool, Vec<(String, f64)>), Box<dyn Error>> {
    let token_mint_pubkey = Pubkey::from_str(mint_address)?;
    let supply = client.get_token_supply(&token_mint_pubkey)?.amount.parse::<f64>()?;
    let largest_accounts = client.get_token_largest_accounts(&token_mint_pubkey)?;
    let mut holders_over_6_percent = 0;
    let mut holders_over_5_percent = 0;
    let mut holders_over_4_percent = 0;
    let mut holders_over_3_percent = 0;
    let mut holders_over_2_percent = 0;
    let mut holders_info = Vec::new();

    println!("前10持有者信息:");
    for (index, account) in largest_accounts.iter().enumerate() {
        if index == 0 {
            continue; // 跳过第一个持有者（池子地址）
        } else if index < 10 {
            let balance: f64 = account.amount.amount.parse()?;
            let percentage = (balance / supply) * 100.0;
            println!("持有者 {} 持仓: {} ({:.2}%)", account.address, balance, percentage);
            holders_info.push((account.address.clone(), percentage));

            if percentage >= 6.0 {
                holders_over_6_percent += 1;
            }else if percentage >= 5.0 {
                holders_over_5_percent += 1;
            }else if percentage >= 4.0 {
                holders_over_4_percent += 1;
            } else if percentage >= 3.0 {
                holders_over_3_percent += 1;
            } else if percentage >= 2.0 {
                holders_over_2_percent += 1;
            }
        } else {
            break;
        }
    }

    // 检查筛选条件，如果 持有2%的地址大于等于5 或 持有3%的地址大于等于3 或 持有4%的地址大于等于3 或 持有5%的地址大于等于2 或 持有6%的地址大于等于1 就返回false，否则返回true
    let meets_condition = !(holders_over_2_percent >= 5 || holders_over_3_percent >= 3 || holders_over_4_percent >= 3 || holders_over_5_percent >= 2 || holders_over_6_percent >= 1);
    Ok((meets_condition, holders_info))
}

fn get_metadata_pubkey(mint: &Pubkey) -> Result<Pubkey, Box<dyn Error>> {
    let metadata_program_id = Pubkey::from_str("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s")?;
    let seeds = &[
        b"metadata",
        metadata_program_id.as_ref(),
        mint.as_ref(),
    ];
    let (metadata_pubkey, _) = Pubkey::find_program_address(seeds, &metadata_program_id);
    Ok(metadata_pubkey)
}

async fn get_token_metadata(client: Arc<RpcClient>, mint_address: &str) -> Result<(String, String), Box<dyn Error>> {
    let mint_pubkey = Pubkey::from_str(mint_address)?;
    let metadata_pubkey = get_metadata_pubkey(&mint_pubkey)?;
    let account_data = client.get_account_data(&metadata_pubkey)?;
    let metadata: Metadata = Metadata::deserialize(&mut &account_data[..])?;
    Ok((metadata.name.trim().to_string(), metadata.symbol.trim().to_string()))
}

#[tokio::main]
async fn main() {
    // Solana RPC 终端
    let client = Arc::new(RpcClient::new(RPC_URL.to_string()));
    let sol_price = Arc::new(Mutex::new(0.0));
    let sol_price_clone = Arc::clone(&sol_price);
    let address_str = "TSLvdd1pWpHVjahSpsvCXUbgwsL3JAcvokwaKt1eokM";
    let address = address_str.parse::<Pubkey>().expect("无效的公钥");
    let keypair = read_keypair_file(KEYPAIR).expect("无法读取密钥对");
    let mut last_known_signature: Option<String> = None;
    let mut monitoring_tokens: HashMap<String, TokenMonitoringInfo> = HashMap::new();
    let mut total_monitoring_token = 0;
    let mut larger_declines_occurred = 0;
    let mut total_buys = 0;
    let mut total_double_sells = 0;
    let mut total_non_double_sells = 0;
    let mut not_within_buying_range = 0;
    let mut hit_keywords = 0;
    let mut distribution_of_holders = 0;

    tokio::spawn(async move {
        if let Err(e) = update_sol_price_in_usdc(sol_price_clone).await {
            println!("更新SOL价格错误: {}", e);
        }
    });

    loop {
        // 获取最新的签名
        let config = GetConfirmedSignaturesForAddress2Config {
            commitment: Some(CommitmentConfig::confirmed()),
            limit: Some(1),
            ..GetConfirmedSignaturesForAddress2Config::default()
        };

        match client.get_signatures_for_address_with_config(&address, config) {
            Ok(signatures) => {
                if let Some(sig_info) = signatures.first() {
                    let sig_str = sig_info.signature.clone();

                    if Some(&sig_str) != last_known_signature.as_ref() {
                        last_known_signature = Some(sig_str.clone());

                        let sig = sig_str.parse::<Signature>().expect("无效的签名");

                        match client.get_transaction_with_config(
                            &sig,
                            RpcTransactionConfig {
                                encoding: Some(solana_transaction_status::UiTransactionEncoding::Json),
                                commitment: Some(CommitmentConfig::confirmed()),
                                max_supported_transaction_version: Some(0),
                            },
                        ) {
                            Ok(tx) => {
                                if let Some(meta) = tx.transaction.meta {
                                    if let Some(post_token_balances) = <OptionSerializer<
                                        Vec<UiTransactionTokenBalance>,
                                    > as Into<Option<Vec<UiTransactionTokenBalance>>>>::into(
                                        meta.post_token_balances,
                                    ) {
                                        for token_balance in post_token_balances {
                                            let mint_address = token_balance.mint.clone();
                                            if !monitoring_tokens.contains_key(&mint_address) {
                                                println!("🌟检测到新的代币铸造地址: {}", mint_address);
                                                total_monitoring_token += 1;
                                                monitoring_tokens.insert(
                                                    mint_address.clone(),
                                                    TokenMonitoringInfo {
                                                        mint_address: mint_address.clone(),
                                                        start_time: Instant::now(),
                                                        purchase_price: None,
                                                        highest_price: 0.0,
                                                        price_history: Vec::new(),
                                                        is_monitoring_phase: true,
                                                        buy_time: None,
                                                        name: None,
                                                        symbol: None,
                                                    },
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            Err(_) => eprintln!("获取交易 {} 时出错", sig_str),
                        }
                    }
                }
            }
            Err(e) => eprintln!("获取签名时出错: {}", e),
        }

        // 定义过滤关键词集合
        let filter_keywords: HashSet<&str> = vec!["eth", "btc", "pump", "dump", "whale", "dev"].into_iter().collect();

        let mint_addresses: Vec<String> = monitoring_tokens.keys().cloned().collect();
        if !mint_addresses.is_empty() {
        let prices = get_token_prices(Arc::clone(&client), &mint_addresses, Arc::clone(&sol_price)).await;

        if let Ok(prices) = prices {
            let mut tokens_to_remove = Vec::new();

            for (mint, token_info) in &mut monitoring_tokens {
                if let Some(current_price) = prices.get(mint) {
                    let elapsed = token_info.start_time.elapsed();
                    let price = *current_price;

                    if token_info.purchase_price.is_none() {
                        token_info.price_history.push(price);
                        
                        if price > token_info.highest_price {
                            token_info.highest_price = price;
                        }

                        if price.is_nan() || price < token_info.highest_price * 0.6 {
                            println!("❌️代币 {} 价格从最高点下跌40%或价格为NaN，停止监控", mint);
                            larger_declines_occurred += 1;
                            tokens_to_remove.push(mint.clone());
                            continue;
                        }
                    }

                    if token_info.is_monitoring_phase {
                        if elapsed < Duration::from_secs(600) {//价格检测期为600秒/10分钟
                            // 检测期，暂时不操作
                         } else {
                            if price > 0.000012 && price < 0.000015 {
                                // 获取代币的名称和符号
                                let metadata_result = get_token_metadata(Arc::clone(&client), mint).await;
                                match metadata_result {
                                    Ok((token_name, token_symbol)) => {
                                        token_info.name = Some(token_name);
                                        token_info.symbol = Some(token_symbol);
                                    }
                                    Err(e) => {
                                        println!("获取代币元数据失败: {}", e);
                                        continue;
                                    }
                                }
                    
                                // 检查持有者分布条件
                                if let (Some(name), Some(symbol)) = (&token_info.name, &token_info.symbol) {
                                    println!("💰💰💰代币 {} 正在进行买入前检查, 持有者分布检查[1/2], 合约:{}", symbol, mint);
                                    
                                    match get_top_holders(Arc::clone(&client), mint).await {
                                        Ok((true, holders_info)) => {
                                            println!("持有者分布符合条件，可以继续");
                    
                                            //进行关键词检查
                                            println!("💰💰💰代币 {} 正在进行买入前检查, 关键词检查[2/2], 合约:{}", symbol, mint);
                                            let contains_keyword = filter_keywords.iter().any(|&kw| 
                                                name.to_lowercase().contains(kw) || symbol.to_lowercase().contains(kw)
                                            );
                    
                                            if contains_keyword {
                                                println!("🟥🟥🟥代币名称: {}, 代币符号: {} 命中关键词，跳过买入", name, symbol);
                                                hit_keywords += 1;
                                                tokens_to_remove.push(mint.clone());
                                                continue;
                                            }
                                            println!("未命中关键词，可以继续");
                    
                                            // 买入操作
                                             println!("正在买入 {} ,合约 {}", symbol, mint);
                                            if let Ok(_) = buy_token(&client, &keypair, mint, 0.1, 15).await {//买入0.1SOL，滑点设置为15%
                                                token_info.purchase_price = Some(price);
                                                token_info.buy_time = Some(Instant::now());
                                                total_buys += 1;
                                                token_info.is_monitoring_phase = false;
                                                println!("🟢🟢🟢已购买代币: {}, 价格: {:.9}", symbol, price);
                    
                                                let holders_message: String = holders_info.iter()
                                                    .enumerate()
                                                    .map(|(i, (_, percentage))| format!("地址{}：{:.2}%", i + 1, percentage))
                                                    .collect::<Vec<String>>()
                                                    .join("\n");
                                                let message_content = format!("✅✅✅已买入\n代币符号: {}\n代币名称: {}\n买入价格: {}\n买入时间: {:?}\n持有信息:\n{}\n[ape.pro](https://ape.pro/solana/{}) - [holder](https://solscan.io/token/{}#holders)", symbol, name, price, token_info.buy_time, holders_message, mint, mint);
                                                let _send = send_discord_message(DISCORD_WEBHOOK_URL, &message_content).await;
                                            }
                                        }
                                        Ok((false, _holders_info)) => {
                                            println!("❗️❗️❗️代币 {} 持有者分布不符合买入条件", symbol);
                                            distribution_of_holders += 1;
                                            tokens_to_remove.push(mint.clone());
                                        }
                                        Err(e) => {
                                            println!("获取持有者信息失败: {}", e);
                                        }
                                    }
                                }
                             } else {
                                println!("🚫代币 {} 价格低于 0.000007，跳过买入", mint);
                                not_within_buying_range += 1;
                                tokens_to_remove.push(mint.clone());
                            }
                        }
                    } else {
                            if let Some(purchase_price) = token_info.purchase_price {
                                let now = Instant::now();
                                if let Some(name) = &token_info.name {
                                    if let Some(symbol) = &token_info.symbol {
                                        println!("🔅\x1b[32m代币 {} | 买入价格:\x1b[33m{:.9} | \x1b[36m当前价格:\x1b[35m{:.9}\x1b[0m | 合约:{}", symbol, purchase_price, price, mint);
                                        if let Some(buy_time) = token_info.buy_time {
                                            let elapsed_after_buy = now.duration_since(buy_time);
                                            if elapsed_after_buy >= Duration::from_secs(1800) {// 买入30分钟内未达到设定价格，卖出
                                                println!("💶💶💶代币 {} 已买入超过30分钟未达到目标价，准备卖出", symbol);
                                                if let Ok(_) = sell_token(&client, &keypair, mint, 1.0, 99).await {//卖出100%，滑点设置为99%
                                                    total_non_double_sells += 1;
                                                    tokens_to_remove.push(mint.clone());
                                                    println!("🟠🟠🟠已卖出代币 {} 全部，价格: {}", symbol, price);
                                                }
                                            }
                                        }
                                    
                                        if price >= purchase_price * 2.0 || price.is_nan() {//当前价格>= 买入价格*2.0，卖出（2.0表示价格上涨200%）
                                            println!("🔥🔥🔥代币 {} 达到目标价格，准备卖出", symbol);
                                            if let Ok(_) = sell_token(&client, &keypair, mint, 1.0, 99).await {//卖出100%，滑点设置为99%
                                                total_double_sells += 1;
                                                tokens_to_remove.push(mint.clone());
                                                println!("🟠🟠🟠已卖出代币 {} ，价格: {}", symbol, price);
                                                let message_content = format!("🔥🔥🔥\n代币符号: {}\n代币名称: {}\n卖出100%\n[ape.pro](https://ape.pro/solana/{})",symbol, name, mint);
                                                let _send = send_discord_message(DISCORD_WEBHOOK_URL, &message_content).await;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                for mint in tokens_to_remove {
                    monitoring_tokens.remove(&mint);
                }
            }
        }

        println!("监控信息： 总监控代币:{} | 正在监控:{} | 不符合价格范围:{} | 发生过较大跌幅:{} | 命中关键词:{} | 发现较大持有者:{}", total_monitoring_token, monitoring_tokens.len(), not_within_buying_range, larger_declines_occurred, hit_keywords, distribution_of_holders);
        println!("买卖信息： 总买入次数:{} | 已达标卖出:{} | 未达标卖出:{}", total_buys, total_double_sells, total_non_double_sells);
        sleep(Duration::from_secs(3)).await;
    }
}

async fn buy_token(
    client: &RpcClient,
    keypair: &Keypair,
    mint_address: &str,
    amount_in_sol: f64,
    slippage: u8,
) -> Result<(), Box<dyn Error>> {
    let mint_pubkey = Pubkey::from_str(mint_address)?;
    let pump_fun_program = Pubkey::from_str(PUMP_FUN_PROGRAM)?;
    let owner = keypair.pubkey();
    let global = Pubkey::from_str("4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf")?;
    let fee_recipient = Pubkey::from_str("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM")?;
    let rent = Pubkey::from_str("SysvarRent111111111111111111111111111111111")?;
    let token_program = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")?;
    let event_authority = Pubkey::from_str("Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1")?;
    let token_accounts = client.get_token_accounts_by_owner(&keypair.pubkey(),TokenAccountsFilter::Mint(mint_pubkey))?;
    if let Some(token_account) = token_accounts.first() {
        if let UiAccountData::Json(parsed_data) = &token_account.account.data {
            if let Some(parsed) = parsed_data.parsed.get("info") {
                if let Some(amount_str) = parsed
                    .get("tokenAmount")
                    .and_then(|v| v.get("amount").and_then(|a| a.as_str()))
                {
                    let amount: u64 = amount_str.parse()?;
                    if amount > 0 {
                        println!("账户已经持有代币 {}，余额为 {}，跳过买入", mint_address, amount);
                        return Ok(());
                    }
                }
            }
        }
    }

    let coin_data = get_coin_data(client, mint_address)?;
    let virtual_sol_reserves = coin_data.virtual_sol_reserves as f64;
    let virtual_token_reserves = coin_data.virtual_token_reserves as f64;
    let sol_in_lamports = (amount_in_sol * LAMPORTS_PER_SOL as f64) as u64;
    let amount = ((sol_in_lamports as f64 * virtual_token_reserves) / virtual_sol_reserves) as u64;
    let slippage_adjustment = 1.0 + (slippage as f64 / 100.0);
    let max_sol_cost = (sol_in_lamports as f64 * slippage_adjustment) as u64;
    let compute_unit_price_instr = ComputeBudgetInstruction::set_compute_unit_price(UNIT_PRICE);
    let compute_unit_limit_instr = ComputeBudgetInstruction::set_compute_unit_limit(UNIT_BUDGET as u32);
    let token_account = spl_associated_token_account::get_associated_token_address(&owner, &mint_pubkey);
    let associated_token_account_instr = if client.get_account(&token_account).is_err() {
        Some(spl_associated_token_account::instruction::create_associated_token_account(&owner, &owner, &mint_pubkey, &token_program))
    } else {
        None
    };

    let mut data = vec![0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea];
    data.write_u64::<LittleEndian>(amount)?;
    data.write_u64::<LittleEndian>(max_sol_cost)?;
    let keys = vec![
        AccountMeta::new(global, false),
        AccountMeta::new(fee_recipient, false),
        AccountMeta::new(mint_pubkey, false),
        AccountMeta::new(coin_data.bonding_curve, false),
        AccountMeta::new(coin_data.associated_bonding_curve, false),
        AccountMeta::new(token_account, false),
        AccountMeta::new(owner, true),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        AccountMeta::new_readonly(token_program, false),
        AccountMeta::new_readonly(rent, false),
        AccountMeta::new_readonly(event_authority, false),
        AccountMeta::new_readonly(pump_fun_program, false),
    ];
    let instruction = Instruction::new_with_bytes(pump_fun_program, &data, keys);
    let mut instructions = vec![compute_unit_price_instr, compute_unit_limit_instr];
    if let Some(instr) = associated_token_account_instr {
        instructions.push(instr);
    }
    instructions.push(instruction);
    let mut transaction = Transaction::new_with_payer(&instructions, Some(&owner));
    transaction.sign(&[keypair], client.get_latest_blockhash()?);
    match client.send_and_confirm_transaction_with_spinner_and_commitment(
        &transaction,
        CommitmentConfig::confirmed(),
    ) {
        Ok(signature) => {
            println!("发送成功，签名: {}", signature);
            Ok(())
        },
        Err(e) => {
            eprintln!("交易发送失败: {:?}", e);
            Err(Box::new(e))
        },
    }
}

async fn sell_token(
    client: &RpcClient,
    keypair: &Keypair,
    mint_address: &str,
    percentage: f64,
    slippage: u8,
) -> Result<(), Box<dyn Error>> {
    let mint_pubkey = Pubkey::from_str(mint_address)?;
    let owner = keypair.pubkey();
    let pump_fun_program = Pubkey::from_str(PUMP_FUN_PROGRAM)?;
    let global = Pubkey::from_str("4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf")?;
    let fee_recipient = Pubkey::from_str("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM")?;
    let token_program = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")?;
    let assoc_token_acc_prog = Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")?;
    let event_authority = Pubkey::from_str("Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1")?;

    let mut retries = 0;
    let max_retries = 3;
    let mut amount: u64 = 0;

    loop {
        let token_accounts = client.get_token_accounts_by_owner(
            &owner,
            TokenAccountsFilter::Mint(mint_pubkey),
        )?;

        if let Some(token_account) = token_accounts.first() {
            if let UiAccountData::Json(parsed_data) = &token_account.account.data {
                if let Some(parsed) = parsed_data.parsed.get("info") {
                    if let Some(amount_str) = parsed
                        .get("tokenAmount")
                        .and_then(|v| v.get("amount").and_then(|a| a.as_str()))
                    {
                        amount = amount_str.parse()?;
                        println!("代币余额: {}", amount);
                        if amount == 0 {
                            println!("账户没有代币 {}，跳过卖出", mint_address);
                            return Ok(());
                        }
                        break;
                    }
                }
            }
        } else {
            retries += 1;
            if retries >= max_retries {
                let error_msg = format!("未找到代币账户 {}，重试次数达到上限，跳过卖出", mint_address);
                return Err(error_msg.into());
            }
            println!("未找到代币账户 {}，等待后重试 ({}/{})", mint_address, retries, max_retries);
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
    }

    let sell_amount = (amount as f64 * percentage) as u64;
    let coin_data = get_coin_data(client, mint_address)?;
    let virtual_sol_reserves = coin_data.virtual_sol_reserves as f64 / 1_000_000_000_f64;
    let virtual_token_reserves = coin_data.virtual_token_reserves as f64 / 1_000_000_f64;
    let token_price = virtual_sol_reserves / virtual_token_reserves;
    let sol_out = (sell_amount as f64 * token_price) as u64;
    let min_sol_output = (sol_out as f64 * (1.0 - slippage as f64 / 100.0)) as u64;

    let compute_unit_price_instr = ComputeBudgetInstruction::set_compute_unit_price(UNIT_PRICE);
    let compute_unit_limit_instr = ComputeBudgetInstruction::set_compute_unit_limit(UNIT_BUDGET as u32);
    let mut data = vec![0x33, 0xe6, 0x85, 0xa4, 0x01, 0x7f, 0x83, 0xad];
    data.write_u64::<LittleEndian>(sell_amount)?;
    data.write_u64::<LittleEndian>(min_sol_output)?;
    let token_account = spl_associated_token_account::get_associated_token_address(&owner, &mint_pubkey);
    let keys = vec![
        AccountMeta::new(global, false),
        AccountMeta::new(fee_recipient, false),
        AccountMeta::new(mint_pubkey, false),
        AccountMeta::new(coin_data.bonding_curve, false),
        AccountMeta::new(coin_data.associated_bonding_curve, false),
        AccountMeta::new(token_account, false),
        AccountMeta::new(owner, true),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        AccountMeta::new_readonly(assoc_token_acc_prog, false),
        AccountMeta::new_readonly(token_program, false),
        AccountMeta::new_readonly(event_authority, false),
        AccountMeta::new_readonly(pump_fun_program, false),
    ];
    
    let instruction = Instruction::new_with_bytes(pump_fun_program, &data, keys);
    let instructions = vec![compute_unit_price_instr, compute_unit_limit_instr, instruction];
    let mut transaction = Transaction::new_with_payer(&instructions, Some(&owner));
    let blockhash = client.get_latest_blockhash()?;
    transaction.sign(&[keypair], blockhash);
    match client.send_and_confirm_transaction_with_spinner_and_commitment(
        &transaction,
        CommitmentConfig::confirmed(),
    ) {
        Ok(signature) => {
            println!("发送成功，签名: {}", signature);
            Ok(())
        },
        Err(e) => {
            eprintln!("交易发送失败: {:?}", e);
            Err(Box::new(e))
        },
    }
}


async fn send_discord_message(webhook_url: &str, content: &str) -> Result<(), Box<dyn Error>> {
    let client = Client::new();
    let payload = json!({ "content": content });
    let response = client
        .post(webhook_url)
        .json(&payload)
        .send()
        .await?;

    if response.status().is_success() {
        //println!("✅ 消息已成功发送到 Discord");
    } else {
        println!("❌ 发送消息到 Discord 失败: {:?}", response.status());
    }

    Ok(())
}
