#[cfg(all(test, feature = "vm-tests"))]
mod tests;

#[cfg(all(test, feature = "vm-tests"))]
mod vault;

#[cfg(all(test, feature = "vm-tests"))]
mod escrow;

#[cfg(all(test, feature = "vm-tests"))]
mod limit_order;

#[cfg(all(test, feature = "vm-tests"))]
mod vesting;

#[cfg(all(test, feature = "vm-tests"))]
mod layout;

#[cfg(all(test, feature = "vm-tests"))]
mod counter_web;

#[cfg(all(test, feature = "vm-tests"))]
mod generic_cells;

#[cfg(all(test, feature = "vm-tests"))]
mod amm;
