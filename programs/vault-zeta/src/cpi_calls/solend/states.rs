use super::*;
use arrayref::{array_mut_ref, array_ref, array_refs, mut_array_refs};
use anchor_lang::solana_program::{
  clock::Slot,
  msg,
  program_error::ProgramError,
  program_pack::{IsInitialized, Pack, Sealed},
  pubkey::{Pubkey, PUBKEY_BYTES},
};
use std::{
  cmp::{Ordering},
};
use std::ops::Deref;
use super::math::Decimal;

/// Percentage of an obligation that can be repaid during each liquidation call
pub const LIQUIDATION_CLOSE_FACTOR: u8 = 20;

/// Obligation borrow amount that is small enough to close out
pub const LIQUIDATION_CLOSE_AMOUNT: u64 = 2;

/// Maximum quote currency value that can be liquidated in 1 liquidate_obligation call
pub const MAX_LIQUIDATABLE_VALUE_AT_ONCE: u64 = 500_000;

/// Number of slots to consider stale after
pub const STALE_AFTER_SLOTS_ELAPSED: u64 = 1;


fn pack_decimal(decimal: Decimal, dst: &mut [u8; 16]) {
  *dst = decimal
    .to_scaled_val()
    .expect("Decimal cannot be packed")
    .to_le_bytes();
}

fn unpack_decimal(src: &[u8; 16]) -> Decimal {
  Decimal::from_scaled_val(u128::from_le_bytes(*src))
}

#[derive(Clone)]
pub struct SolendProgram;

impl anchor_lang::Id for SolendProgram {
  fn id() -> Pubkey {
    solend_program::ID
  }
}

/// Last update state
#[derive(Clone, Debug, Default)]
pub struct LastUpdate {
  /// Last slot when updated
  pub slot: Slot,
  /// True when marked stale, false when slot updated
  pub stale: bool,
}

impl LastUpdate {
  /// Create new last update
  pub fn new(slot: Slot) -> Self {
    Self { slot, stale: true }
  }

  /// Return slots elapsed since given slot
  pub fn slots_elapsed(&self, slot: Slot) -> Result<u64, ProgramError> {
    let slots_elapsed = slot
      .checked_sub(self.slot)
      .unwrap();
    Ok(slots_elapsed)
  }

  /// Set last update slot
  pub fn update_slot(&mut self, slot: Slot) {
    self.slot = slot;
    self.stale = false;
  }

  /// Set stale to true
  pub fn mark_stale(&mut self) {
    self.stale = true;
  }

  /// Check if marked stale or last update slot is too long ago
  pub fn is_stale(&self, slot: Slot) -> Result<bool, ProgramError> {
    Ok(self.stale || self.slots_elapsed(slot)? >= STALE_AFTER_SLOTS_ELAPSED)
  }
}

impl PartialEq for LastUpdate {
  fn eq(&self, other: &Self) -> bool {
    self.slot == other.slot
  }
}

impl PartialOrd for LastUpdate {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    self.slot.partial_cmp(&other.slot)
  }
}

/// Reserve liquidity
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReserveLiquidity {
  /// Reserve liquidity mint address
  pub mint_pubkey: Pubkey,
  /// Reserve liquidity mint decimals
  pub mint_decimals: u8,
  /// Reserve liquidity supply address
  pub supply_pubkey: Pubkey,
  /// Reserve liquidity pyth oracle account
  pub pyth_oracle_pubkey: Pubkey,
  /// Reserve liquidity switchboard oracle account
  pub switchboard_oracle_pubkey: Pubkey,
  /// Reserve liquidity available
  pub available_amount: u64,
  /// Reserve liquidity borrowed
  pub borrowed_amount_wads: Decimal,
  /// Reserve liquidity cumulative borrow rate
  pub cumulative_borrow_rate_wads: Decimal,
  /// Reserve cumulative protocol fees
  pub accumulated_protocol_fees_wads: Decimal,
  /// Reserve liquidity market price in quote currency
  pub market_price: Decimal,
}

/// Reserve collateral
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReserveCollateral {
  /// Reserve collateral mint address
  pub mint_pubkey: Pubkey,
  /// Reserve collateral mint supply, used for exchange rate
  pub mint_total_supply: u64,
  /// Reserve collateral supply address
  pub supply_pubkey: Pubkey,
}

/// Reserve configuration values
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReserveConfig {
  /// Optimal utilization rate, as a percentage
  pub optimal_utilization_rate: u8,
  /// Target ratio of the value of borrows to deposits, as a percentage
  /// 0 if use as collateral is disabled
  pub loan_to_value_ratio: u8,
  /// Bonus a liquidator gets when repaying part of an unhealthy obligation, as a percentage
  pub liquidation_bonus: u8,
  /// Loan to value ratio at which an obligation can be liquidated, as a percentage
  pub liquidation_threshold: u8,
  /// Min borrow APY
  pub min_borrow_rate: u8,
  /// Optimal (utilization) borrow APY
  pub optimal_borrow_rate: u8,
  /// Max borrow APY
  pub max_borrow_rate: u8,
  /// Program owner fees assessed, separate from gains due to interest accrual
  pub fees: ReserveFees,
  /// Maximum deposit limit of liquidity in native units, u64::MAX for inf
  pub deposit_limit: u64,
  /// Borrows disabled
  pub borrow_limit: u64,
  /// Reserve liquidity fee receiver address
  pub fee_receiver: Pubkey,
  /// Cut of the liquidation bonus that the protocol receives, as a percentage
  pub protocol_liquidation_fee: u8,
  /// Protocol take rate is the amount borrowed interest protocol recieves, as a percentage
  pub protocol_take_rate: u8,
}

/// Additional fee information on a reserve
///
/// These exist separately from interest accrual fees, and are specifically for the program owner
/// and frontend host. The fees are paid out as a percentage of liquidity token amounts during
/// repayments and liquidations.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReserveFees {
  /// Fee assessed on `BorrowObligationLiquidity`, expressed as a Wad.
  /// Must be between 0 and 10^18, such that 10^18 = 1.  A few examples for
  /// clarity:
  /// 1% = 10_000_000_000_000_000
  /// 0.01% (1 basis point) = 100_000_000_000_000
  /// 0.00001% (Aave borrow fee) = 100_000_000_000
  pub borrow_fee_wad: u64,
  /// Fee for flash loan, expressed as a Wad.
  /// 0.3% (Aave flash loan fee) = 3_000_000_000_000_000
  pub flash_loan_fee_wad: u64,
  /// Amount of fee going to host account, if provided in liquidate and repay
  pub host_fee_percentage: u8,
}

/// Calculate fees exlusive or inclusive of an amount
pub enum FeeCalculation {
  /// Fee added to amount: fee = rate * amount
  Exclusive,
  /// Fee included in amount: fee = (rate / (1 + rate)) * amount
  Inclusive,
}

/// Lending market reserve state
#[repr(C)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReserveState {
  /// Version of the struct
  pub version: u8,
  /// Last slot when supply and rates updated
  pub last_update: LastUpdate,
  /// Lending market address
  pub lending_market: Pubkey,
  /// Reserve liquidity
  pub liquidity: ReserveLiquidity,
  /// Reserve collateral
  pub collateral: ReserveCollateral,
  /// Reserve configuration values
  pub config: ReserveConfig,
}


#[derive(Clone)]
pub struct Reserve(ReserveState);

impl Sealed for ReserveState {}

impl IsInitialized for ReserveState {
  fn is_initialized(&self) -> bool {
    self.version != 0
  }
}

impl Pack for ReserveState {
  const LEN: usize = 619;

  // @TODO: break this up by reserve / liquidity / collateral / config https://git.io/JOCca
  fn pack_into_slice(&self, output: &mut [u8]) {
    let output = array_mut_ref![output, 0, 619];
    #[allow(clippy::ptr_offset_with_cast)]
      let (
      version,
      last_update_slot,
      last_update_stale,
      lending_market,
      liquidity_mint_pubkey,
      liquidity_mint_decimals,
      liquidity_supply_pubkey,
      liquidity_pyth_oracle_pubkey,
      liquidity_switchboard_oracle_pubkey,
      liquidity_available_amount,
      liquidity_borrowed_amount_wads,
      liquidity_cumulative_borrow_rate_wads,
      liquidity_market_price,
      collateral_mint_pubkey,
      collateral_mint_total_supply,
      collateral_supply_pubkey,
      config_optimal_utilization_rate,
      config_loan_to_value_ratio,
      config_liquidation_bonus,
      config_liquidation_threshold,
      config_min_borrow_rate,
      config_optimal_borrow_rate,
      config_max_borrow_rate,
      config_fees_borrow_fee_wad,
      config_fees_flash_loan_fee_wad,
      config_fees_host_fee_percentage,
      config_deposit_limit,
      config_borrow_limit,
      config_fee_receiver,
      config_protocol_liquidation_fee,
      config_protocol_take_rate,
      liquidity_accumulated_protocol_fees_wads,
      _padding,
    ) = mut_array_refs![
            output,
            1,
            8,
            1,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            1,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            8,
            16,
            16,
            16,
            PUBKEY_BYTES,
            8,
            PUBKEY_BYTES,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            8,
            8,
            1,
            8,
            8,
            PUBKEY_BYTES,
            1,
            1,
            16,
            230
        ];

    // reserve
    *version = self.version.to_le_bytes();
    *last_update_slot = self.last_update.slot.to_le_bytes();
    *last_update_stale = if self.last_update.stale { [1_u8] } else { [0_u8] };
    lending_market.copy_from_slice(self.lending_market.as_ref());

    // liquidity
    liquidity_mint_pubkey.copy_from_slice(self.liquidity.mint_pubkey.as_ref());
    *liquidity_mint_decimals = self.liquidity.mint_decimals.to_le_bytes();
    liquidity_supply_pubkey.copy_from_slice(self.liquidity.supply_pubkey.as_ref());
    liquidity_pyth_oracle_pubkey.copy_from_slice(self.liquidity.pyth_oracle_pubkey.as_ref());
    liquidity_switchboard_oracle_pubkey
      .copy_from_slice(self.liquidity.switchboard_oracle_pubkey.as_ref());
    *liquidity_available_amount = self.liquidity.available_amount.to_le_bytes();
    pack_decimal(
      self.liquidity.borrowed_amount_wads,
      liquidity_borrowed_amount_wads,
    );
    pack_decimal(
      self.liquidity.cumulative_borrow_rate_wads,
      liquidity_cumulative_borrow_rate_wads,
    );
    pack_decimal(
      self.liquidity.accumulated_protocol_fees_wads,
      liquidity_accumulated_protocol_fees_wads,
    );
    pack_decimal(self.liquidity.market_price, liquidity_market_price);

    // collateral
    collateral_mint_pubkey.copy_from_slice(self.collateral.mint_pubkey.as_ref());
    *collateral_mint_total_supply = self.collateral.mint_total_supply.to_le_bytes();
    collateral_supply_pubkey.copy_from_slice(self.collateral.supply_pubkey.as_ref());

    // config
    *config_optimal_utilization_rate = self.config.optimal_utilization_rate.to_le_bytes();
    *config_loan_to_value_ratio = self.config.loan_to_value_ratio.to_le_bytes();
    *config_liquidation_bonus = self.config.liquidation_bonus.to_le_bytes();
    *config_liquidation_threshold = self.config.liquidation_threshold.to_le_bytes();
    *config_min_borrow_rate = self.config.min_borrow_rate.to_le_bytes();
    *config_optimal_borrow_rate = self.config.optimal_borrow_rate.to_le_bytes();
    *config_max_borrow_rate = self.config.max_borrow_rate.to_le_bytes();
    *config_fees_borrow_fee_wad = self.config.fees.borrow_fee_wad.to_le_bytes();
    *config_fees_flash_loan_fee_wad = self.config.fees.flash_loan_fee_wad.to_le_bytes();
    *config_fees_host_fee_percentage = self.config.fees.host_fee_percentage.to_le_bytes();
    *config_deposit_limit = self.config.deposit_limit.to_le_bytes();
    *config_borrow_limit = self.config.borrow_limit.to_le_bytes();
    config_fee_receiver.copy_from_slice(self.config.fee_receiver.as_ref());
    *config_protocol_liquidation_fee = self.config.protocol_liquidation_fee.to_le_bytes();
    *config_protocol_take_rate = self.config.protocol_take_rate.to_le_bytes();
  }

  /// Unpacks a byte buffer into a [ReserveInfo](struct.ReserveInfo.html).
  fn unpack_from_slice(input: &[u8]) -> Result<Self, ProgramError> {
    let input = array_ref![input, 0, 619];
    #[allow(clippy::ptr_offset_with_cast)]
      let (
      version,
      last_update_slot,
      last_update_stale,
      lending_market,
      liquidity_mint_pubkey,
      liquidity_mint_decimals,
      liquidity_supply_pubkey,
      liquidity_pyth_oracle_pubkey,
      liquidity_switchboard_oracle_pubkey,
      liquidity_available_amount,
      liquidity_borrowed_amount_wads,
      liquidity_cumulative_borrow_rate_wads,
      liquidity_market_price,
      collateral_mint_pubkey,
      collateral_mint_total_supply,
      collateral_supply_pubkey,
      config_optimal_utilization_rate,
      config_loan_to_value_ratio,
      config_liquidation_bonus,
      config_liquidation_threshold,
      config_min_borrow_rate,
      config_optimal_borrow_rate,
      config_max_borrow_rate,
      config_fees_borrow_fee_wad,
      config_fees_flash_loan_fee_wad,
      config_fees_host_fee_percentage,
      config_deposit_limit,
      config_borrow_limit,
      config_fee_receiver,
      config_protocol_liquidation_fee,
      config_protocol_take_rate,
      liquidity_accumulated_protocol_fees_wads,
      _padding,
    ) = array_refs![
            input,
            1,
            8,
            1,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            1,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            PUBKEY_BYTES,
            8,
            16,
            16,
            16,
            PUBKEY_BYTES,
            8,
            PUBKEY_BYTES,
            1,
            1,
            1,
            1,
            1,
            1,
            1,
            8,
            8,
            1,
            8,
            8,
            PUBKEY_BYTES,
            1,
            1,
            16,
            230
        ];

    let version = u8::from_le_bytes(*version);
    if version > 1 {
      msg!("Reserve version does not match lending program version");
      return Err(ProgramError::InvalidAccountData);
    }

    Ok(Self {
      version,
      last_update: LastUpdate {
        slot: u64::from_le_bytes(*last_update_slot),
        stale: last_update_stale.first().unwrap() == &1_u8,
      },
      lending_market: Pubkey::new_from_array(*lending_market),
      liquidity: ReserveLiquidity {
        mint_pubkey: Pubkey::new_from_array(*liquidity_mint_pubkey),
        mint_decimals: u8::from_le_bytes(*liquidity_mint_decimals),
        supply_pubkey: Pubkey::new_from_array(*liquidity_supply_pubkey),
        pyth_oracle_pubkey: Pubkey::new_from_array(*liquidity_pyth_oracle_pubkey),
        switchboard_oracle_pubkey: Pubkey::new_from_array(
          *liquidity_switchboard_oracle_pubkey,
        ),
        available_amount: u64::from_le_bytes(*liquidity_available_amount),
        borrowed_amount_wads: unpack_decimal(liquidity_borrowed_amount_wads),
        cumulative_borrow_rate_wads: unpack_decimal(liquidity_cumulative_borrow_rate_wads),
        accumulated_protocol_fees_wads: unpack_decimal(
          liquidity_accumulated_protocol_fees_wads,
        ),
        market_price: unpack_decimal(liquidity_market_price),
      },
      collateral: ReserveCollateral {
        mint_pubkey: Pubkey::new_from_array(*collateral_mint_pubkey),
        mint_total_supply: u64::from_le_bytes(*collateral_mint_total_supply),
        supply_pubkey: Pubkey::new_from_array(*collateral_supply_pubkey),
      },
      config: ReserveConfig {
        optimal_utilization_rate: u8::from_le_bytes(*config_optimal_utilization_rate),
        loan_to_value_ratio: u8::from_le_bytes(*config_loan_to_value_ratio),
        liquidation_bonus: u8::from_le_bytes(*config_liquidation_bonus),
        liquidation_threshold: u8::from_le_bytes(*config_liquidation_threshold),
        min_borrow_rate: u8::from_le_bytes(*config_min_borrow_rate),
        optimal_borrow_rate: u8::from_le_bytes(*config_optimal_borrow_rate),
        max_borrow_rate: u8::from_le_bytes(*config_max_borrow_rate),
        fees: ReserveFees {
          borrow_fee_wad: u64::from_le_bytes(*config_fees_borrow_fee_wad),
          flash_loan_fee_wad: u64::from_le_bytes(*config_fees_flash_loan_fee_wad),
          host_fee_percentage: u8::from_le_bytes(*config_fees_host_fee_percentage),
        },
        deposit_limit: u64::from_le_bytes(*config_deposit_limit),
        borrow_limit: u64::from_le_bytes(*config_borrow_limit),
        fee_receiver: Pubkey::new_from_array(*config_fee_receiver),
        protocol_liquidation_fee: u8::from_le_bytes(*config_protocol_liquidation_fee),
        protocol_take_rate: u8::from_le_bytes(*config_protocol_take_rate),
      },
    })
  }
}


impl anchor_lang::Owner for Reserve {
  fn owner() -> Pubkey {
    solend_program::ID
  }
}

impl anchor_lang::AccountSerialize for Reserve {}

impl anchor_lang::AccountDeserialize for Reserve {
  fn try_deserialize_unchecked(buf: &mut &[u8]) -> anchor_lang::prelude::Result<Self> {
    Ok(ReserveState::unpack(buf).map(Reserve)?)
  }
}

impl Deref for Reserve {
  type Target = ReserveState;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}
