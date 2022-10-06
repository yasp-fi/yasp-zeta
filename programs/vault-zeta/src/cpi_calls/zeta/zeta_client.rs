use super::zeta_context::*;
use super::*;
use cpi_interface::global_interface;
use anchor_lang::prelude::*;

/// Zeta Program Client
/// Defines a clean interface and set of helper functions to make CPI calls to the Zeta Program

#[global_interface]
pub trait ZetaInterface<'info, T: Accounts<'info>> {
    fn initialize_margin_account(ctx: Context<T>) -> Result<()>;
    fn deposit(ctx: Context<T>, amount: u64) -> Result<()>;
    fn withdraw(ctx: Context<T>, amount: u64) -> Result<()>;
    fn initialize_open_orders(ctx: Context<T>) -> Result<()>;
    fn place_order(
        ctx: Context<T>,
        price: u64,
        size: u64,
        side: Side,
        client_order_id: Option<u64>,
    ) -> Result<()>;
    fn place_order_v3(
        ctx: Context<T>,
        price: u64,
        size: u64,
        side: Side,
        order_type: OrderType,
        client_order_id: Option<u64>,
        tag: Option<String>,
    ) -> Result<()>;
    fn cancel_order(ctx: Context<T>, side: Side, order_id: u128) -> Result<()>;
}

pub fn initialize_margin_account<'info>(
    zeta_program: AccountInfo<'info>,
    cpi_accounts: InitializeMarginAccount<'info>,
    seeds: &[&[u8]],
) -> Result<()> {
    let signer = &[&seeds[..]];
    let cpi_ctx = CpiContext::new_with_signer(zeta_program, cpi_accounts, signer);
    zeta_interface::initialize_margin_account(cpi_ctx)
}

pub fn deposit<'info>(
    zeta_program: AccountInfo<'info>,
    cpi_accounts: Deposit<'info>,
    amount: u64,
    seeds: &[&[u8]],
) -> Result<()> {
    let signer = &[&seeds[..]];
    let cpi_ctx = CpiContext::new_with_signer(zeta_program, cpi_accounts, signer);
    zeta_interface::deposit(cpi_ctx, amount)
}

pub fn withdraw<'info>(
    zeta_program: AccountInfo<'info>,
    cpi_accounts: Withdraw<'info>,
    amount: u64,
    seeds: &[&[u8]],
) -> Result<()> {
    let signer = &[&seeds[..]];
    let cpi_ctx = CpiContext::new_with_signer(zeta_program, cpi_accounts, signer);
    zeta_interface::withdraw(cpi_ctx, amount)
}

pub fn initialize_open_orders<'info>(
    zeta_program: AccountInfo<'info>,
    cpi_accounts: InitializeOpenOrders<'info>,
    seeds: &[&[u8]],
) -> Result<()> {
    let signer = &[&seeds[..]];
    let cpi_ctx = CpiContext::new_with_signer(zeta_program, cpi_accounts, signer);
    zeta_interface::initialize_open_orders(cpi_ctx)
}

pub fn place_order<'info>(
    zeta_program: AccountInfo<'info>,
    cpi_accounts: PlaceOrder<'info>,
    price: u64,
    size: u64,
    side: Side,
    client_order_id: Option<u64>,
    seeds: &[&[u8]],
) -> Result<()> {
    let signer = &[&seeds[..]];
    let cpi_ctx = CpiContext::new_with_signer(zeta_program, cpi_accounts, signer);
    zeta_interface::place_order(cpi_ctx, price, size, side, client_order_id)
}

pub fn place_order_v3<'info>(
    zeta_program: AccountInfo<'info>,
    cpi_accounts: PlaceOrder<'info>,
    price: u64,
    size: u64,
    side: Side,
    order_type: OrderType,
    client_order_id: Option<u64>,
    tag: Option<String>, // Not stored, only used when sniffing the transactions
    seeds: &[&[u8]],
) -> Result<()> {
    let signer = &[&seeds[..]];
    let cpi_ctx = CpiContext::new_with_signer(zeta_program, cpi_accounts, signer);
    zeta_interface::place_order_v3(cpi_ctx, price, size, side, order_type, client_order_id, tag)
}

pub fn cancel_order<'info>(
    zeta_program: AccountInfo<'info>,
    cpi_accounts: CancelOrder<'info>,
    side: Side,
    order_id: u128,
    seeds: &[&[u8]],
) -> Result<()> {
    let signer = &[&seeds[..]];
    let cpi_ctx = CpiContext::new_with_signer(zeta_program, cpi_accounts, signer);
    zeta_interface::cancel_order(cpi_ctx, side, order_id)
}
