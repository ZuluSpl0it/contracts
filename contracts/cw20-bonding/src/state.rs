use cosmwasm_schema::cw_serde;

use cosmwasm_std::{Addr, Uint128};
use crate::item::Item;

use crate::curves::DecimalPlaces;
use crate::msg::CurveType;

/// Supply is dynamic and tracks the current supply of staked and ERC20 tokens.
#[cw_serde]
pub struct CurveState {
    /// reserve is how many native tokens exist bonded to the validator
    pub reserve: Uint128,
    /// supply is how many tokens this contract has issued
    pub supply: Uint128,

    // the denom of the reserve token
    pub reserve_denom: String,

    // how to normalize reserve and supply
    pub decimals: DecimalPlaces,

    // How much tax collected since start
    pub tax_collected: Uint128,
}

impl CurveState {
    pub fn new(reserve_denom: String, decimals: DecimalPlaces) -> Self {
        CurveState {
            reserve: Uint128::zero(),
            supply: Uint128::zero(),
            reserve_denom,
            decimals,
            tax_collected: Uint128::zero(),
        }
    }
}

#[cw_serde]
pub struct Config {
    pub owner: Addr,
    pub can_buy: String,    
    pub can_sell: String,
    pub devs_acct: String,
    pub burn_acct: String,
    pub raffle_acct: String,
    pub social_acct: String,
    pub expense_acct: String,
}

pub const CURVE_STATE: Item<CurveState> = Item::new("curve_state");

pub const CURVE_TYPE: Item<CurveType> = Item::new("curve_type");

pub const CONFIG: Item<Config> = Item::new("config");
