use super::*;
use anchor_lang::prelude::*;
use std::cell::RefMut;
use std::convert::{TryFrom, TryInto};
use std::ops::DerefMut;

#[macro_export]
macro_rules! wrap_error {
    ($err:expr) => {{
        msg!("Error thrown at {}:{}", file!(), line!());
        $err
    }};
}

#[error_code]
pub enum FuzeErrorCode {
  #[msg("Account not mutable")]
  AccountNotMutable,
  #[msg("Unsupported kind")]
  UnsupportedKind,
  #[msg("Product strike uninitialized")]
  ProductStrikeUninitialized,
  #[msg("Invalid product market key")]
  InvalidProductMarketKey,
  #[msg("Market not live")]
  MarketNotLive,
  #[msg("Product dirty")]
  ProductDirty,
  #[msg("Invalid option kind, must be Call or Put")]
  InvalidOptionKind,
}

pub fn deserialize_account_info_zerocopy<'a, T: bytemuck::Pod>(
    account_info: &'a AccountInfo,
) -> Result<RefMut<'a, T>> {
    let data = account_info.try_borrow_mut_data()?;
    Ok(RefMut::map(data, |data| {
        bytemuck::from_bytes_mut(&mut data.deref_mut()[8..])
    }))
}

#[inline(never)]
pub fn deserialize_account_info<'a, T: AccountSerialize + AccountDeserialize + Owner + Clone>(
    account_info: &AccountInfo<'a>,
) -> Result<T> {
    let mut data: &[u8] = &account_info.try_borrow_data()?;
    Ok(T::try_deserialize_unchecked(&mut data)?)
}

pub fn get_otm_amount(spot: u64, strike: u64, product: Kind) -> Result<u64> {
    match product {
        Kind::Call => Ok((strike as i128)
            .checked_sub(spot as i128)
            .unwrap()
            .max(0)
            .try_into()
            .unwrap()),
        Kind::Put => Ok((spot as i128)
            .checked_sub(strike as i128)
            .unwrap()
            .max(0)
            .try_into()
            .unwrap()),
        _ => return wrap_error!(Err(error!(FuzeErrorCode::UnsupportedKind))),
    }
}

/// Initial margin for single product
pub fn get_initial_margin_per_lot(
    spot: u64,
    strike: u64,
    mark: u64,
    product: Kind,
    side: Side,
    margin_parameters: &MarginParameters,
) -> Result<u64> {
    let initial_margin: u128 = match product {
        Kind::Future => (spot as u128)
            .checked_mul(margin_parameters.future_margin_initial.into())
            .unwrap()
            .checked_div(NATIVE_PRECISION_DENOMINATOR)
            .unwrap(),
        Kind::Call | Kind::Put => match side {
            Side::Bid => (spot as u128)
                .checked_mul(margin_parameters.option_spot_percentage_long_initial.into())
                .unwrap()
                .checked_div(NATIVE_PRECISION_DENOMINATOR)
                .unwrap()
                .min(
                    (mark as u128)
                        .checked_mul(margin_parameters.option_mark_percentage_long_initial.into())
                        .unwrap()
                        .checked_div(NATIVE_PRECISION_DENOMINATOR)
                        .unwrap(),
                ),
            Side::Ask => {
                let otm_amount: u128 = get_otm_amount(spot, strike, product)?.into();
                let otm_pct = otm_amount
                    .checked_mul(NATIVE_PRECISION_DENOMINATOR)
                    .unwrap()
                    .checked_div(spot.into())
                    .unwrap();

                let dynamic_margin_pct = (margin_parameters.option_dynamic_percentage_short_initial
                    as u128)
                    .checked_sub(otm_pct)
                    .unwrap_or(0);

                let margin_pct = dynamic_margin_pct.max(
                    margin_parameters
                        .option_spot_percentage_short_initial
                        .into(),
                );
                margin_pct
                    .checked_mul(spot.into())
                    .unwrap()
                    .checked_div(NATIVE_PRECISION_DENOMINATOR)
                    .unwrap()
            }
            Side::Uninitialized => unreachable!(),
        },
        _ => return wrap_error!(Err(error!(FuzeErrorCode::UnsupportedKind))),
    };

    if product == Kind::Put && side == Side::Ask {
        let sell_put_cap_margin = (strike as u128)
            .checked_mul(margin_parameters.option_short_put_cap_percentage as u128)
            .unwrap()
            .checked_div(NATIVE_PRECISION_DENOMINATOR)
            .unwrap();

        return Ok(u64::try_from(initial_margin.min(sell_put_cap_margin)).unwrap());
    }

    Ok(u64::try_from(initial_margin).unwrap())
}

/// Maintenance margin for single product
pub fn get_maintenance_margin_per_lot(
    spot: u64,
    strike: u64,
    mark: u64,
    product: Kind,
    long: bool,
    margin_parameters: &MarginParameters,
) -> Result<u64> {
    let maintenance_margin: u128 = match product {
        Kind::Future => (spot as u128)
            .checked_mul(margin_parameters.future_margin_maintenance.into())
            .unwrap()
            .checked_div(NATIVE_PRECISION_DENOMINATOR)
            .unwrap(),
        Kind::Call | Kind::Put => {
            if long {
                (spot as u128)
                    .checked_mul(
                        margin_parameters
                            .option_spot_percentage_long_maintenance
                            .into(),
                    )
                    .unwrap()
                    .checked_div(NATIVE_PRECISION_DENOMINATOR)
                    .unwrap()
                    .min(
                        (mark as u128)
                            .checked_mul(
                                margin_parameters
                                    .option_mark_percentage_long_maintenance
                                    .into(),
                            )
                            .unwrap()
                            .checked_div(NATIVE_PRECISION_DENOMINATOR)
                            .unwrap(),
                    )
            } else {
                let otm_amount: u128 = get_otm_amount(spot, strike, product)?.into();
                let otm_pct = otm_amount
                    .checked_mul(NATIVE_PRECISION_DENOMINATOR)
                    .unwrap()
                    .checked_div(spot.into())
                    .unwrap();

                let dynamic_margin_pct: u128 =
                    (margin_parameters.option_dynamic_percentage_short_maintenance as u128)
                        .checked_sub(otm_pct)
                        .unwrap_or(0);

                let margin_pct = dynamic_margin_pct.max(
                    margin_parameters
                        .option_spot_percentage_short_maintenance
                        .into(),
                );
                margin_pct
                    .checked_mul(spot.into())
                    .unwrap()
                    .checked_div(NATIVE_PRECISION_DENOMINATOR)
                    .unwrap()
            }
        }
        _ => return wrap_error!(Err(error!(FuzeErrorCode::UnsupportedKind))),
    };

    if product == Kind::Put && !long {
        let sell_put_cap_margin = (strike as u128)
            .checked_mul(margin_parameters.option_short_put_cap_percentage as u128)
            .unwrap()
            .checked_div(NATIVE_PRECISION_DENOMINATOR)
            .unwrap();

        return Ok(u64::try_from(maintenance_margin.min(sell_put_cap_margin)).unwrap());
    }

    Ok(u64::try_from(maintenance_margin).unwrap())
}

/// Returns the native oracle price (6.dp)
///
/// # Arguments
///
/// * `oracle` - Oracle account.
pub fn get_native_oracle_price(oracle: &AccountInfo) -> u64 {
    let oracle_price = pyth_client::Price::load(&oracle).unwrap();
    (oracle_price.agg.price as u128)
        .checked_mul(10u128.pow(PLATFORM_PRECISION.into()))
        .unwrap()
        .checked_div(10u128.pow((-oracle_price.expo).try_into().unwrap()))
        .unwrap()
        .try_into()
        .unwrap()
}

pub fn get_oracle_price(oracle: &AccountInfo, precision: u32) -> i128 {
    let oracle_price = pyth_client::Price::load(&oracle).unwrap();
    (oracle_price.agg.price as u128)
        .checked_mul(10u128.pow(precision))
        .unwrap()
        .checked_div(10u128.pow((-oracle_price.expo).try_into().unwrap()))
        .unwrap()
        .try_into()
        .unwrap()
}

/// Returns the market index given an expiry index and index into the slice.
///
/// # Arguments
///
/// * `expiry_index` - Expiry series index.
/// * `product_index` - Index into the products slice. [0..NUM_PRODUCTS_PER_SERIES).
pub fn get_products_slice_market_index(expiry_index: usize, product_index: usize) -> usize {
    expiry_index
        .checked_mul(NUM_PRODUCTS_PER_SERIES)
        .unwrap()
        .checked_add(product_index)
        .unwrap()
}
