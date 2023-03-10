use cosmwasm_std::{
    attr, to_binary, Attribute, CanonicalAddr, Coin, CosmosMsg, Decimal, DepsMut, Env, MessageInfo,
    Response, StdError, StdResult, Uint128, WasmMsg,
};

use crate::{
    bond::deposit_farm_share,
    model::ExecuteMsg,
    querier::{
        astroport_router_simulate_swap, query_astroport_pending_token, query_astroport_pool_balance,
    },
    state::{read_config, state_store},
};

use cw20::Cw20ExecuteMsg;

use crate::bond::deposit_farm2_share;
use crate::state::{pool_info_read, pool_info_store, read_state, Config, PoolInfo};
use astroport::asset::{Asset, AssetInfo};
use astroport::generator::{
    Cw20HookMsg as AstroportCw20HookMsg, ExecuteMsg as AstroportExecuteMsg,
};
use astroport::pair::{
    Cw20HookMsg as AstroportPairCw20HookMsg, ExecuteMsg as AstroportPairExecuteMsg,
};
use astroport::querier::{query_token_balance, simulate};
use astroport::router::{ExecuteMsg as AstroportRouterExecuteMsg, SwapOperation};
use moneymarket::market::ExecuteMsg as MoneyMarketExecuteMsg;
use spectrum_protocol::farm_helper::deduct_tax;
use spectrum_protocol::gov::ExecuteMsg as GovExecuteMsg;
use spectrum_protocol::gov_proxy::Cw20HookMsg as GovProxyCw20HookMsg;
pub fn compound(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    threshold_compound_astro: Uint128,
) -> StdResult<Response> {
    let config = read_config(deps.storage)?;

    if config.controller != deps.api.addr_canonicalize(info.sender.as_str())? {
        return Err(StdError::generic_err("unauthorized"));
    }

    let stasset_token = deps.api.addr_humanize(&config.stasset_token)?;
    let stluna_token = deps.api.addr_humanize(&config.stluna_token)?;
    let weldo_token = deps.api.addr_humanize(&config.weldo_token)?;
    let astro_token = deps.api.addr_humanize(&config.astro_token)?;
    let xastro_proxy = deps.api.addr_humanize(&config.xastro_proxy)?;
    let astro_ust_pair_contract = deps.api.addr_humanize(&config.astro_ust_pair_contract)?;
    let pair_contract = deps.api.addr_humanize(&config.pair_contract)?;
    let astroport_router = deps.api.addr_humanize(&config.astroport_router)?;

    let uluna = "uluna".to_string();
    let uusd = "uusd".to_string();

    let mut pool_info = pool_info_read(deps.storage).load(config.stasset_token.as_slice())?;

    // This get pending (ASTRO), and pending proxy rewards
    let pending_token_response = query_astroport_pending_token(
        deps.as_ref(),
        &pool_info.staking_token,
        &env.contract.address,
        &config.astroport_generator,
    )?;

    let lp_balance = query_astroport_pool_balance(
        deps.as_ref(),
        &pool_info.staking_token,
        &env.contract.address,
        &config.astroport_generator,
    )?;

    let mut total_weldo_token_swap_amount = Uint128::zero();
    let mut total_weldo_token_stake_amount = Uint128::zero();
    let mut total_weldo_token_commission = Uint128::zero();
    let mut total_astro_token_swap_amount = Uint128::zero();
    let mut total_astro_token_stake_amount = Uint128::zero();
    let mut total_astro_token_commission = Uint128::zero();
    let mut compound_amount_astro = Uint128::zero();

    let mut attributes: Vec<Attribute> = vec![];
    let community_fee = config.community_fee;
    let platform_fee = config.platform_fee;
    let controller_fee = config.controller_fee;
    let total_fee = community_fee + platform_fee + controller_fee;

    let reward = query_token_balance(
        &deps.querier,
        weldo_token.clone(),
        env.contract.address.clone(),
    )? + pending_token_response
        .pending_on_proxy
        .unwrap_or_else(Uint128::zero);
    let reward_astro = query_token_balance(
        &deps.querier,
        astro_token.clone(),
        env.contract.address.clone(),
    )? + pending_token_response.pending;

    // calculate auto-compound, auto-stake, and commission in astro token
    let mut state = read_state(deps.storage)?;
    if !reward_astro.is_zero() && !lp_balance.is_zero() && reward_astro > threshold_compound_astro {
        let commission_astro = reward_astro * total_fee;
        let astro_amount = reward_astro.checked_sub(commission_astro)?;
        // add commission to total swap amount
        total_astro_token_commission += commission_astro;
        total_astro_token_swap_amount += commission_astro;

        let auto_bond_amount_astro = lp_balance.checked_sub(pool_info.total_stake_bond_amount)?;
        compound_amount_astro = astro_amount.multiply_ratio(auto_bond_amount_astro, lp_balance);
        let stake_amount_astro = astro_amount.checked_sub(compound_amount_astro)?;

        attributes.push(attr("commission_astro", commission_astro));
        attributes.push(attr("compound_amount_astro", compound_amount_astro));
        attributes.push(attr("stake_amount_astro", stake_amount_astro));

        total_astro_token_stake_amount += stake_amount_astro;

        deposit_farm_share(
            deps.as_ref(),
            &env,
            &mut state,
            &mut pool_info,
            &config,
            total_astro_token_stake_amount,
        )?;
    }

    // calculate auto-compound, auto-stake, and commission in farm token
    if !reward.is_zero() && !lp_balance.is_zero() {
        let commission = reward * total_fee;
        let weldo_token_amount = reward.checked_sub(commission)?;
        // add commission to total swap amount
        total_weldo_token_commission += commission;

        let auto_bond_amount = lp_balance.checked_sub(pool_info.total_stake_bond_amount)?;
        let compound_amount = weldo_token_amount.multiply_ratio(auto_bond_amount, lp_balance);
        let stake_amount = weldo_token_amount.checked_sub(compound_amount)?;
        total_weldo_token_swap_amount += commission + compound_amount;

        attributes.push(attr("commission", commission));
        attributes.push(attr("compound_amount", compound_amount));
        attributes.push(attr("stake_amount", stake_amount));

        total_weldo_token_stake_amount += stake_amount;

        deposit_farm2_share(
            deps.as_ref(),
            &env,
            &mut state,
            &mut pool_info,
            &config,
            total_weldo_token_stake_amount,
        )?;
    }

    state_store(deps.storage).save(&state)?;
    pool_info_store(deps.storage).save(config.stasset_token.as_slice(), &pool_info)?;

    // swap all
    total_astro_token_swap_amount += compound_amount_astro;
    let (total_ust_reinvest_amount_astro, total_ust_commission_amount_astro) =
        if !total_astro_token_swap_amount.is_zero() {
            // find ASTRO swap rate
            let astro_asset = Asset {
                info: AssetInfo::Token {
                    contract_addr: astro_token.clone(),
                },
                amount: total_astro_token_swap_amount,
            };
            let astro_swap_rate =
                simulate(&deps.querier, astro_ust_pair_contract.clone(), &astro_asset)?;
            let total_ust_return_amount_astro =
                deduct_tax(&deps.querier, astro_swap_rate.return_amount, uusd.clone())?;
            attributes.push(attr(
                "total_ust_return_amount_astro",
                total_ust_return_amount_astro,
            ));

            let total_ust_commission_amount_astro = total_ust_return_amount_astro
                .multiply_ratio(total_astro_token_commission, total_astro_token_swap_amount);

            let total_ust_reinvest_amount_astro =
                total_ust_return_amount_astro.checked_sub(total_ust_commission_amount_astro)?;

            (total_ust_reinvest_amount_astro, total_ust_commission_amount_astro)
        } else {
            (Uint128::zero(), Uint128::zero())
        };

    let weldo_token_swap_rate = astroport_router_simulate_swap(
        deps.as_ref(),
        total_weldo_token_swap_amount,
        vec![
            SwapOperation::AstroSwap {
                offer_asset_info: AssetInfo::Token {
                    contract_addr: weldo_token.clone(),
                },
                ask_asset_info: AssetInfo::Token {
                    contract_addr: stluna_token.clone(),
                },
            },
            SwapOperation::AstroSwap {
                offer_asset_info: AssetInfo::Token {
                    contract_addr: stluna_token.clone(),
                },
                ask_asset_info: AssetInfo::NativeToken {
                    denom: uluna.clone(),
                },
            },
            SwapOperation::AstroSwap {
                offer_asset_info: AssetInfo::NativeToken {
                    denom: uluna.clone(),
                },
                ask_asset_info: AssetInfo::NativeToken {
                    denom: uusd.clone(),
                },
            },
        ],
        &config.astroport_router,
    )?;

    let total_weldo_ust_return_amount =
        deduct_tax(&deps.querier, weldo_token_swap_rate.amount, uusd.clone())?;
    attributes.push(attr(
        "total_weldo_ust_return_amount",
        total_weldo_ust_return_amount,
    ));

    let total_ust_commission_amount = if total_weldo_token_swap_amount != Uint128::zero() {
        total_weldo_ust_return_amount
            .multiply_ratio(total_weldo_token_commission, total_weldo_token_swap_amount)
    } else {
        Uint128::zero()
    };
    let total_weldo_ust_reinvest_amount =
        total_weldo_ust_return_amount.checked_sub(total_ust_commission_amount)?;

    let total_ust_reinvest_amount =
        total_weldo_ust_reinvest_amount + total_ust_reinvest_amount_astro;

    let net_swap = total_ust_reinvest_amount.multiply_ratio(1000u128, 1997u128);
    let net_liquidity = total_ust_reinvest_amount.checked_sub(net_swap)?;

    let net_swap_after_tax = deduct_tax(&deps.querier, net_swap, uusd.clone())?;
    let net_liquidity_after_tax = deduct_tax(&deps.querier, net_liquidity, uusd.clone())?;

    // find farm token swap rate
    let net_swap_asset = Asset {
        info: AssetInfo::NativeToken {
            denom: uusd.clone(),
        },
        amount: net_swap_after_tax,
    };
    let swap_rate = simulate(&deps.querier, pair_contract.clone(), &net_swap_asset)?;

    let mut messages: Vec<CosmosMsg> = vec![];

    let manual_claim_pending_token = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: deps
            .api
            .addr_humanize(&config.astroport_generator)?
            .to_string(),
        funds: vec![],
        msg: to_binary(&AstroportExecuteMsg::Withdraw {
            lp_token: deps.api.addr_humanize(&pool_info.staking_token)?,
            amount: Uint128::zero(),
        })?,
    });
    messages.push(manual_claim_pending_token);

    if !total_astro_token_swap_amount.is_zero() {
        let swap_astro_token: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: astro_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: astro_ust_pair_contract.to_string(),
                amount: total_astro_token_swap_amount,
                msg: to_binary(&AstroportPairCw20HookMsg::Swap {
                    max_spread: Some(Decimal::percent(50)),
                    belief_price: None,
                    to: None,
                })?,
            })?,
            funds: vec![],
        });
        messages.push(swap_astro_token);
    }
    if !total_weldo_token_swap_amount.is_zero() {
        let swap_weldo_token: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: weldo_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: astroport_router.to_string(),
                amount: total_weldo_token_swap_amount,
                msg: to_binary(&AstroportRouterExecuteMsg::ExecuteSwapOperations {
                    operations: vec![
                        SwapOperation::AstroSwap {
                            offer_asset_info: AssetInfo::Token {
                                contract_addr: weldo_token.clone(),
                            },
                            ask_asset_info: AssetInfo::Token {
                                contract_addr: stluna_token.clone(),
                            },
                        },
                        SwapOperation::AstroSwap {
                            offer_asset_info: AssetInfo::Token {
                                contract_addr: stluna_token,
                            },
                            ask_asset_info: AssetInfo::NativeToken {
                                denom: uluna.clone(),
                            },
                        },
                        SwapOperation::AstroSwap {
                            offer_asset_info: AssetInfo::NativeToken { denom: uluna },
                            ask_asset_info: AssetInfo::NativeToken {
                                denom: uusd.clone(),
                            },
                        },
                    ],
                    minimum_receive: None,
                    to: None,
                    max_spread: Some(Decimal::percent(50)),
                })?,
            })?,
            funds: vec![],
        });
        messages.push(swap_weldo_token);
    }

    if !total_ust_reinvest_amount.is_zero() {
        let swap_ust: CosmosMsg = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pair_contract.to_string(),
            msg: to_binary(&AstroportPairExecuteMsg::Swap {
                offer_asset: net_swap_asset,
                max_spread: Some(Decimal::percent(50)),
                belief_price: None,
                to: None,
            })?,
            funds: vec![Coin {
                denom: uusd.clone(),
                amount: net_swap_after_tax,
            }],
        });
        messages.push(swap_ust);
    }

    if let Some(gov_proxy) = config.gov_proxy {
        if !total_weldo_token_stake_amount.is_zero() {
            let stake_weldo_token = CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: weldo_token.to_string(),
                funds: vec![],
                msg: to_binary(&Cw20ExecuteMsg::Send {
                    contract: deps.api.addr_humanize(&gov_proxy)?.to_string(),
                    amount: total_weldo_token_stake_amount,
                    msg: to_binary(&GovProxyCw20HookMsg::Stake {})?,
                })?,
            });
            messages.push(stake_weldo_token);
        }
    }

    if !total_astro_token_stake_amount.is_zero() {
        let stake_astro_token = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: astro_token.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: xastro_proxy.to_string(),
                amount: total_astro_token_stake_amount,
                msg: to_binary(&GovProxyCw20HookMsg::Stake {})?,
            })?,
        });
        messages.push(stake_astro_token);
    }

    if !net_liquidity_after_tax.is_zero() {
        let increase_allowance = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: stasset_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::IncreaseAllowance {
                spender: pair_contract.to_string(),
                amount: swap_rate.return_amount,
                expires: None,
            })?,
            funds: vec![],
        });
        messages.push(increase_allowance);

        let provide_liquidity = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: pair_contract.to_string(),
            msg: to_binary(&AstroportPairExecuteMsg::ProvideLiquidity {
                assets: [
                    Asset {
                        info: AssetInfo::Token {
                            contract_addr: stasset_token,
                        },
                        amount: swap_rate.return_amount,
                    },
                    Asset {
                        info: AssetInfo::NativeToken {
                            denom: uusd.clone(),
                        },
                        amount: net_liquidity_after_tax,
                    },
                ],
                slippage_tolerance: Some(Decimal::percent(50)),
                receiver: None,
                auto_stake: Some(true),
            })?,
            funds: vec![Coin {
                denom: uusd.clone(),
                amount: net_liquidity_after_tax,
            }],
        });
        messages.push(provide_liquidity);

        // let stake = CosmosMsg::Wasm(WasmMsg::Execute {
        //     contract_addr: env.contract.address.to_string(),
        //     msg: to_binary(&ExecuteMsg::stake {
        //         asset_token: farm_token.to_string(),
        //     })?,
        //     funds: vec![],
        // });
        // messages.push(stake);
    }

    let total_ust_commission_amount =
        total_ust_commission_amount + total_ust_commission_amount_astro;
    if !total_ust_commission_amount.is_zero() {
        let net_commission_amount =
            deduct_tax(&deps.querier, total_ust_commission_amount, uusd.clone())?;

        let mut state = read_state(deps.storage)?;
        state.earning += net_commission_amount;
        state_store(deps.storage).save(&state)?;

        attributes.push(attr("net_commission", net_commission_amount));

        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.addr_humanize(&config.anchor_market)?.to_string(),
            msg: to_binary(&MoneyMarketExecuteMsg::DepositStable {})?,
            funds: vec![Coin {
                denom: uusd,
                amount: net_commission_amount,
            }],
        }));
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: deps.api.addr_humanize(&config.spectrum_gov)?.to_string(),
            msg: to_binary(&GovExecuteMsg::mint {})?,
            funds: vec![],
        }));
        messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: env.contract.address.to_string(),
            msg: to_binary(&ExecuteMsg::send_fee {})?,
            funds: vec![],
        }));
    }

    attributes.push(attr("action", "compound"));
    attributes.push(attr("provide_farm_token", swap_rate.return_amount));
    attributes.push(attr("provide_ust_amount", net_liquidity_after_tax));

    Ok(Response::new()
        .add_messages(messages)
        .add_attributes(attributes))
}

pub fn stake(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    asset_token: String,
) -> StdResult<Response> {
    // only anchor farm contract can execute this message
    if info.sender != env.contract.address {
        return Err(StdError::generic_err("unauthorized"));
    }
    let config: Config = read_config(deps.storage)?;
    let astroport_generator = deps.api.addr_humanize(&config.astroport_generator)?;
    let asset_token_raw: CanonicalAddr = deps.api.addr_canonicalize(&asset_token)?;
    let pool_info: PoolInfo = pool_info_read(deps.storage).load(asset_token_raw.as_slice())?;
    let staking_token = deps.api.addr_humanize(&pool_info.staking_token)?;

    let amount = query_token_balance(&deps.querier, staking_token.clone(), env.contract.address)?;

    Ok(Response::new()
        .add_messages(vec![CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: staking_token.to_string(),
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: astroport_generator.to_string(),
                amount,
                msg: to_binary(&AstroportCw20HookMsg::Deposit {})?,
            })?,
        })])
        .add_attributes(vec![
            attr("action", "stake"),
            attr("asset_token", asset_token),
            attr("staking_token", staking_token),
            attr("amount", amount),
        ]))
}

pub fn send_fee(deps: DepsMut, env: Env, info: MessageInfo) -> StdResult<Response> {
    // only farm contract can execute this message
    if info.sender != env.contract.address {
        return Err(StdError::generic_err("unauthorized"));
    }
    let config = read_config(deps.storage)?;
    let aust_token = deps.api.addr_humanize(&config.aust_token)?;
    let spectrum_gov = deps.api.addr_humanize(&config.spectrum_gov)?;

    let aust_balance = query_token_balance(
        &deps.querier,
        aust_token.clone(),
        env.contract.address,
    )?;

    let mut messages: Vec<CosmosMsg> = vec![];
    let thousand = Uint128::from(1000u64);
    let total_fee = config.community_fee + config.controller_fee + config.platform_fee;
    let community_amount = aust_balance.multiply_ratio(thousand * config.community_fee, thousand * total_fee);
    if !community_amount.is_zero() {
        let transfer_community_fee = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: aust_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: spectrum_gov.to_string(),
                amount: community_amount,
            })?,
            funds: vec![],
        });
        messages.push(transfer_community_fee);
    }

    let platform_amount = aust_balance.multiply_ratio(thousand * config.platform_fee, thousand * total_fee);
    if !platform_amount.is_zero() {
        let stake_platform_fee = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: aust_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: deps.api.addr_humanize(&config.platform)?.to_string(),
                amount: platform_amount,
            })?,
            funds: vec![],
        });
        messages.push(stake_platform_fee);
    }

    let controller_amount = aust_balance.checked_sub(community_amount + platform_amount)?;
    if !controller_amount.is_zero() {
        let stake_controller_fee = CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr: aust_token.to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient: deps.api.addr_humanize(&config.controller)?.to_string(),
                amount: controller_amount,
            })?,
            funds: vec![],
        });
        messages.push(stake_controller_fee);
    }
    Ok(Response::new()
        .add_messages(messages))
}
