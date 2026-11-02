use ckb_contract_core::{
    ckb_hash, entity_id,
    token::{decode_amount, AmountEncoding, PlainXudtAsset},
    BinaryCodec, Error, StateTransition, CKB,
};
use ckb_contract_testkit::{
    self as kit,
    ckb_testtool::{
        ckb_script::{ScriptError, TransactionScriptError},
        ckb_types::{
            bytes::Bytes,
            core::{ScriptHashType, TransactionBuilder, TransactionView},
            packed::{OutPoint, Script},
            prelude::*,
        },
        context::Context,
    },
};
use ckb_contract_tx::{
    context::{CellSet, ContextTx, ResolvedLayout},
    edit::with_action,
    input, Cell,
};
use framework_amm_script::{cells::PoolArgs, Action, State, LAYOUT, MINIMUM_LIQUIDITY};

const CYCLES: u64 = 80_000_000;
const FUNDING: u128 = 10_000_000;

struct Fixture {
    context: Context,
    layout: ResolvedLayout,
    args: PoolArgs,
    wallets: [Script; 3],
    assets: [PlainXudtAsset; 2],
    funding: Vec<OutPoint>,
    points: Option<CellSet>,
    state: State,
    lp: [Option<OutPoint>; 3],
}

impl Fixture {
    fn new(extended_x: bool, extended_y: bool) -> Self {
        let root = kit::workspace_root(env!("CARGO_MANIFEST_DIR"));
        let mut context = Context::default();
        let code = context.deploy_cell(kit::load_binary(&root, "framework-amm-script"));
        let base = context
            .build_script_with_hash_type(&code, ScriptHashType::Data2, Bytes::new())
            .unwrap();
        let assets = [
            PlainXudtAsset {
                owner: [9; 32],
                extended_args: extended_x,
            },
            PlainXudtAsset {
                owner: [8; 32],
                extended_args: extended_y,
            },
        ];
        let types: Vec<_> = assets
            .iter()
            .map(|asset| kit::deploy_xudt(&mut context, &root, *asset).1)
            .collect();
        let wallets = kit::wallets(&mut context);
        let funding: Vec<_> = types
            .iter()
            .map(|asset| {
                let cell = Cell::token(&wallets[0], asset, 5000 * CKB, FUNDING);
                context.create_cell(cell.output, cell.data)
            })
            .collect();
        let id = entity_id(input(funding[0].clone()).as_slice().try_into().unwrap(), 0);
        let layout = ResolvedLayout::new(&LAYOUT, &base, &id, &types).unwrap();
        let state = State::new(
            ckb_hash(wallets[0].as_slice()),
            assets[0],
            assets[1],
            1_000_000,
            4_000_000,
        )
        .unwrap();
        Self {
            context,
            layout,
            args: PoolArgs::new(&base, &id),
            wallets,
            assets,
            funding,
            points: None,
            state,
            lp: [None, None, None],
        }
    }

    fn creation(&self) -> TransactionView {
        ContextTx::create(
            &self.layout,
            &self.state,
            &[self.state.reserve_x, self.state.reserve_y],
        )
        .unwrap()
        .input(input(self.funding[0].clone()))
        .input(input(self.funding[1].clone()))
        .output(Cell::token(
            &self.wallets[0],
            &self.args.lp_type,
            200 * CKB,
            self.state.total_shares - MINIMUM_LIQUIDITY,
        ))
        .output(
            self.layout
                .token(
                    0,
                    &self.wallets[0],
                    200 * CKB,
                    FUNDING - self.state.reserve_x,
                )
                .unwrap(),
        )
        .output(
            self.layout
                .token(
                    1,
                    &self.wallets[0],
                    200 * CKB,
                    FUNDING - self.state.reserve_y,
                )
                .unwrap(),
        )
        .output(Cell::capacity(&self.wallets[0], 8599 * CKB))
        .build()
        .unwrap()
    }

    fn start(&mut self) {
        self.commit(self.creation());
    }

    fn commit(&mut self, tx: TransactionView) -> TransactionView {
        let tx = kit::assert_valid(&mut self.context, tx, CYCLES);
        for point in &mut self.lp {
            if point
                .as_ref()
                .is_some_and(|point| tx.input_pts_iter().any(|input| &input == point))
            {
                *point = None;
            }
        }
        kit::commit_outputs(&mut self.context, &tx);
        self.points = Some(CellSet::at(&LAYOUT, tx.hash()).unwrap());
        self.state = State::decode(&tx.outputs_data().get(0).unwrap().raw_data()).unwrap();
        for (index, output) in tx.outputs().into_iter().enumerate() {
            if output.type_().to_opt().as_ref() == Some(&self.args.lp_type) {
                let wallet = self
                    .wallets
                    .iter()
                    .position(|wallet| wallet == &output.lock())
                    .unwrap();
                self.lp[wallet] = Some(
                    OutPoint::new_builder()
                        .tx_hash(tx.hash())
                        .index(index as u32)
                        .build(),
                );
            }
        }
        tx
    }

    fn hash(&self, wallet: usize) -> [u8; 32] {
        ckb_hash(self.wallets[wallet].as_slice())
    }

    fn swap(&self, wallet: usize, reverse: bool) -> Action {
        if reverse {
            Action::SwapYForX {
                trader: self.hash(wallet),
                amount_in: 40_000,
                min_out: 1,
            }
        } else {
            Action::SwapXForY {
                trader: self.hash(wallet),
                amount_in: 10_000,
                min_out: 1,
            }
        }
    }

    fn add(&self, wallet: usize) -> Action {
        Action::AddLiquidity {
            provider: self.hash(wallet),
            shares: 200_000,
            max_x: 1_000_000,
            max_y: 1_000_000,
        }
    }

    fn remove(&self, wallet: usize, shares: u128) -> Action {
        Action::RemoveLiquidity {
            provider: self.hash(wallet),
            shares,
            min_x: 1,
            min_y: 1,
        }
    }

    fn update(&mut self, action: Action, state_index: usize) -> TransactionView {
        let flow = self.state.settlement(&action).unwrap();
        let wallet = self
            .wallets
            .iter()
            .position(|wallet| ckb_hash(wallet.as_slice()) == flow.actor)
            .unwrap();
        let actor = self.wallets[wallet].clone();
        let after = self.state.apply(&action).unwrap();
        let mut builder = ContextTx::update(
            &self.layout,
            self.points.clone().unwrap(),
            &after,
            &[after.reserve_x, after.reserve_y],
            &action,
        )
        .unwrap();
        for reserve in 0..2 {
            let cell = self
                .layout
                .token(reserve, &actor, 2000 * CKB, FUNDING)
                .unwrap();
            builder = builder.input(input(self.context.create_cell(cell.output, cell.data)));
        }
        let mut lp_before = 0;
        if let Some(point) = &self.lp[wallet] {
            let (_, data) = self.context.get_cell(point).unwrap();
            lp_before = decode_amount(&data, AmountEncoding::Exact16).unwrap();
            builder = builder.input(input(point.clone()));
        }
        let lp_after = lp_before.checked_sub(flow.burned).unwrap() + flow.minted;
        builder = builder
            .state_input_index(state_index)
            .output(
                self.layout
                    .token(
                        0,
                        &actor,
                        200 * CKB,
                        FUNDING - flow.spent_x + flow.received_x,
                    )
                    .unwrap(),
            )
            .output(
                self.layout
                    .token(
                        1,
                        &actor,
                        200 * CKB,
                        FUNDING - flow.spent_y + flow.received_y,
                    )
                    .unwrap(),
            );
        if lp_after > 0 {
            builder = builder.output(Cell::token(&actor, &self.args.lp_type, 200 * CKB, lp_after));
        }
        builder
            .output(Cell::capacity(
                &actor,
                (4000 + u64::from(lp_before > 0) * 200 - 400 - u64::from(lp_after > 0) * 200 - 1)
                    * CKB,
            ))
            .build()
            .unwrap()
    }

    fn reject(&mut self, tx: TransactionView, error: Error) {
        kit::assert_type_error(&mut self.context, tx, &self.args.state_type, error, CYCLES);
    }
}

#[test]
fn lifecycle_supports_multiple_providers_both_swap_directions_and_real_predecessors() {
    for extended_x in [false, true] {
        for extended_y in [false, true] {
            let mut f = Fixture::new(extended_x, extended_y);
            f.start();
            for (wallet, reverse, position) in [(1, false, 0), (2, true, 1), (0, false, 2)] {
                let tx = f.update(f.swap(wallet, reverse), position);
                let previous = f.points.clone().unwrap();
                f.commit(tx);
                assert!(f.context.get_cell(&previous.state).is_none());
                for reserve in previous.reserves {
                    assert!(f.context.get_cell(&reserve).is_none());
                }
            }
            let tx = f.update(f.add(1), 1);
            f.commit(tx);
            assert!(f.lp[1].is_some());
            let tx = f.update(f.remove(1, 200_000), 2);
            f.commit(tx);
            assert!(f.lp[1].is_none());
            let shares = f.state.total_shares - MINIMUM_LIQUIDITY;
            let tx = f.update(f.remove(0, shares), 1);
            f.commit(tx);
            assert_eq!(f.state.total_shares, MINIMUM_LIQUIDITY);
            assert!(f.state.reserve_x > 0 && f.state.reserve_y > 0);
            let tx = f.update(f.add(2), 0);
            f.commit(tx);
            assert_eq!(f.state.nonce, 7);
        }
    }
}

#[test]
fn lp_transfers_conserve_supply_without_consuming_pool_and_recipient_can_redeem() {
    let mut f = Fixture::new(false, true);
    f.start();
    let point = f.lp[0].take().unwrap();
    let shares = f.state.total_shares - MINIMUM_LIQUIDITY;
    let transfer = TransactionBuilder::default()
        .input(input(point))
        .output(Cell::token(&f.wallets[1], &f.args.lp_type, 199 * CKB, shares).output)
        .output_data(kit::amount(shares).pack())
        .build();
    let tx = kit::assert_valid(&mut f.context, transfer, CYCLES);
    kit::commit_outputs(&mut f.context, &tx);
    f.lp[1] = Some(
        OutPoint::new_builder()
            .tx_hash(tx.hash())
            .index(0u32)
            .build(),
    );
    let tx = f.update(f.remove(1, shares), 1);
    f.commit(tx);
    assert_eq!(f.state.total_shares, MINIMUM_LIQUIDITY);
}

// The pinned official xUDT returns ERROR_AMOUNT when supply increases without
// owner authorization. Do not accept a cycle limit or missing-dependency error.
fn reject_xudt_mint(context: &mut Context, tx: TransactionView) {
    let tx = context.complete_tx(tx);
    let error = context
        .verify_tx(&tx, CYCLES)
        .expect_err("unauthorized xUDT mint accepted");
    let cause = error
        .root_cause()
        .downcast_ref::<TransactionScriptError>()
        .unwrap_or_else(|| panic!("expected xUDT script rejection: {error:?}"));
    assert!(
        matches!(cause.script_error(), ScriptError::ValidationFailure(_, -52)),
        "{error:?}"
    );
}

#[test]
fn lp_is_official_xudt_and_standalone_mint_requires_its_pool_state_type() {
    let mut f = Fixture::new(false, true);
    f.start();
    assert_eq!(
        f.args.lp_type.code_hash().as_slice(),
        ckb_contract_core::token::XUDT_CODE_HASH
    );
    assert_eq!(f.args.lp_type.hash_type(), ScriptHashType::Data1.into());
    let args = f.args.lp_type.args().raw_data();
    assert_eq!(args.len(), 36);
    assert_eq!(&args[..32], ckb_hash(f.args.state_type.as_slice()));
    assert_eq!(&args[32..], &0xe000_0000u32.to_le_bytes());
    let cell = Cell::capacity(&f.wallets[1], 1000 * CKB);
    let funding = f.context.create_cell(cell.output, cell.data);
    let cell = Cell::token(&f.wallets[1], &f.args.lp_type, 999 * CKB, 1);
    let tx = TransactionBuilder::default()
        .input(input(funding))
        .output(cell.output)
        .output_data(cell.data.pack())
        .build();
    reject_xudt_mint(&mut f.context, tx);
}

#[test]
fn standalone_xudt_burn_abandons_claims_without_changing_pool_accounting() {
    let mut f = Fixture::new(false, true);
    f.start();
    let state_before = f.state.clone();
    let points_before = f.points.clone().unwrap();
    let burned = 200_000;
    let remaining = f.state.total_shares - MINIMUM_LIQUIDITY - burned;
    let cell = Cell::token(&f.wallets[0], &f.args.lp_type, 199 * CKB, remaining);
    let tx = TransactionBuilder::default()
        .input(input(f.lp[0].take().unwrap()))
        .output(cell.output)
        .output_data(cell.data.pack())
        .build();
    let tx = kit::assert_valid(&mut f.context, tx, CYCLES);
    kit::commit_outputs(&mut f.context, &tx);
    f.lp[0] = Some(
        OutPoint::new_builder()
            .tx_hash(tx.hash())
            .index(0u32)
            .build(),
    );
    assert_eq!(
        State::decode(&f.context.get_cell(&points_before.state).unwrap().1).unwrap(),
        state_before
    );
    // Only the still-existing LP may be redeemed; voluntarily burned claims
    // remain in the denominator and cannot be recreated by a later depositor.
    let tx = f.update(f.remove(0, remaining), 1);
    f.commit(tx);
    assert_eq!(f.state.total_shares, MINIMUM_LIQUIDITY + burned);
    assert_eq!((f.state.reserve_x, f.state.reserve_y), (100_500, 402_000));
    let tx = f.update(f.add(1), 0);
    f.commit(tx);
    assert_eq!(f.state.total_shares, MINIMUM_LIQUIDITY + burned + 200_000);
}

#[test]
fn creation_rejects_wrong_lp_supply_recipient_initial_state_and_id() {
    for mode in 0..5 {
        let mut f = Fixture::new(false, true);
        let mut tx = f.creation();
        tx = match mode {
            0 => kit::replace_data(tx, 3, kit::amount(f.state.total_shares)),
            1 => {
                let output = tx
                    .outputs()
                    .get(3)
                    .unwrap()
                    .as_builder()
                    .lock(f.wallets[1].clone())
                    .build();
                kit::replace_output(tx, 3, output)
            }
            2 | 3 => {
                let mut state = f.state.clone();
                if mode == 2 {
                    state.nonce = 1;
                } else {
                    state.total_shares += 1;
                }
                kit::replace_data(tx, 0, Bytes::from(state.encode()))
            }
            _ => {
                // Reordering the funding inputs changes the creation ID.
                let mut inputs: Vec<_> = tx.inputs().into_iter().collect();
                inputs.swap(0, 1);
                tx.as_advanced_builder().set_inputs(inputs).build()
            }
        };
        f.reject(
            tx,
            if mode < 2 {
                Error::Token
            } else {
                Error::Genesis
            },
        );
    }
}

#[test]
fn swaps_reject_wrong_recipient_actor_slippage_and_state_tampering() {
    for mode in 0..8 {
        let mut f = Fixture::new(false, true);
        f.start();
        let action = f.swap(1, false);
        let mut tx = f.update(action, 1);
        tx = match mode {
            0 => {
                let output = tx
                    .outputs()
                    .get(4)
                    .unwrap()
                    .as_builder()
                    .lock(f.wallets[2].clone())
                    .build();
                kit::replace_output(tx, 4, output)
            }
            1 => with_action(tx, 1, &f.swap(2, false)).unwrap(),
            2 => with_action(
                tx,
                1,
                &Action::SwapXForY {
                    trader: f.hash(1),
                    amount_in: 10_000,
                    min_out: 39_487,
                },
            )
            .unwrap(),
            _ => {
                let mut after = f.state.apply(&action).unwrap();
                match mode {
                    3 => after.reserve_y -= 1,
                    4 => after.total_shares += 1,
                    5 => after.creator = f.hash(2),
                    6 => after.asset_y.extended_args = false,
                    _ => after.nonce = 0,
                }
                kit::replace_data(tx, 0, Bytes::from(after.encode()))
            }
        };
        f.reject(
            tx,
            match mode {
                0 => Error::Token,
                1 => Error::Authorization,
                _ => Error::Transition,
            },
        );
    }
}

#[test]
fn liquidity_checks_both_asset_payments_lp_supply_and_lp_recipient() {
    for mode in 0..5 {
        let mut f = Fixture::new(false, true);
        f.start();
        let mut tx = f.update(f.add(1), 1);
        tx = match mode {
            0 | 1 => kit::replace_data(
                tx,
                3 + mode,
                kit::amount(if mode == 0 { 9_900_001 } else { 9_600_001 }),
            ),
            2 => kit::replace_data(tx, 5, kit::amount(200_001)),
            3 => {
                let output = tx
                    .outputs()
                    .get(5)
                    .unwrap()
                    .as_builder()
                    .lock(f.wallets[2].clone())
                    .build();
                kit::replace_output(tx, 5, output)
            }
            _ => with_action(
                tx,
                1,
                &Action::AddLiquidity {
                    provider: f.hash(1),
                    shares: 200_000,
                    max_x: 99_999,
                    max_y: 400_000,
                },
            )
            .unwrap(),
        };
        f.reject(
            tx,
            if mode == 4 {
                Error::Transition
            } else {
                Error::Token
            },
        );
    }
    for reserve in 0..2 {
        let mut f = Fixture::new(false, true);
        f.start();
        let action = f.remove(0, 200_000);
        let flow = f.state.settlement(&action).unwrap();
        let tx = f.update(action, 2);
        let received = [flow.received_x, flow.received_y][reserve];
        let tx = kit::replace_data(tx, 3 + reserve, kit::amount(FUNDING + received - 1));
        f.reject(tx, Error::Token);
    }
}

#[test]
fn each_reserve_requires_exact_asset_amount_encoding_capacity_position_and_origin() {
    for reserve in 0..2 {
        for mode in 0..7 {
            let mut f = Fixture::new(false, true);
            f.start();
            let mut tx = f.update(f.swap(1, false), 1);
            let position = 1 + reserve;
            tx = match mode {
                0 => kit::replace_data(tx, position, kit::amount(1)),
                1 => {
                    let mut data = tx.outputs_data().get(position).unwrap().raw_data().to_vec();
                    data.push(0);
                    kit::replace_data(tx, position, Bytes::from(data))
                }
                2 => {
                    let output = tx
                        .outputs()
                        .get(position)
                        .unwrap()
                        .as_builder()
                        .type_(Some(f.layout.reserves()[1 - reserve].token_type.clone()).pack())
                        .build();
                    kit::replace_output(tx, position, output)
                }
                3 => {
                    let output = tx
                        .outputs()
                        .get(position)
                        .unwrap()
                        .as_builder()
                        .capacity(201 * CKB)
                        .build();
                    kit::replace_output(tx, position, output)
                }
                4 => {
                    let mut outputs: Vec<_> = tx.outputs().into_iter().collect();
                    let mut data: Vec<_> = tx.outputs_data().into_iter().collect();
                    outputs.swap(1, 2);
                    data.swap(1, 2);
                    tx.as_advanced_builder()
                        .set_outputs(outputs)
                        .set_outputs_data(data)
                        .build()
                }
                5 => {
                    let original = f.points.as_ref().unwrap().reserves[reserve].clone();
                    let (output, data) = f.context.get_cell(&original).unwrap();
                    let foreign = OutPoint::new_builder()
                        .tx_hash([42; 32].pack())
                        .index(position as u32)
                        .build();
                    f.context
                        .create_cell_with_out_point(foreign.clone(), output, data);
                    let mut inputs: Vec<_> = tx.inputs().into_iter().collect();
                    inputs[2 + reserve] = input(foreign);
                    tx.as_advanced_builder().set_inputs(inputs).build()
                }
                _ => {
                    let output = tx
                        .outputs()
                        .get(position)
                        .unwrap()
                        .as_builder()
                        .lock(f.wallets[1].clone())
                        .build();
                    kit::replace_output(tx, position, output)
                }
            };
            f.reject(
                tx,
                match mode {
                    0..=2 => Error::Token,
                    3 => Error::Capacity,
                    _ => Error::Structure,
                },
            );
        }
    }
}

#[test]
fn action_witness_tracks_actual_state_input_and_rejects_trailing_bytes() {
    for position in 0..=2 {
        let mut f = Fixture::new(false, true);
        f.start();
        let action = f.swap(1, false);
        let tx = f.update(action, position);
        let mut bytes = action.encode();
        bytes.push(0);
        let tx = ckb_contract_tx::edit::with_action_bytes(tx, position, &bytes).unwrap();
        f.reject(tx, Error::Encoding);
    }
}

#[test]
fn reserve_public_lock_cannot_authorize_unrelated_xudt_mint() {
    let mut f = Fixture::new(false, true);
    f.start();
    let tx = f.update(f.swap(1, false), 1);
    let mut owner_mint = f.assets[0];
    owner_mint.owner = ckb_hash(f.args.reserve_x_lock.as_slice());
    let root = kit::workspace_root(env!("CARGO_MANIFEST_DIR"));
    let (_, token) = kit::deploy_xudt(&mut f.context, &root, owner_mint);
    let tx = tx
        .as_advanced_builder()
        .output(Cell::token(&f.wallets[1], &token, 200 * CKB, 1).output)
        .output_data(kit::amount(1).pack())
        .build();
    f.reject(tx, Error::Token);
}

#[test]
fn other_pool_lp_type_cannot_mint_by_referencing_this_pool_state() {
    let mut f = Fixture::new(false, true);
    f.start();
    let tx = f.update(f.swap(1, false), 1);
    let other = PoolArgs::new(&f.args.state_type, &[42; 32]).lp_type;
    let tx = tx
        .as_advanced_builder()
        .output(Cell::token(&f.wallets[1], &other, 200 * CKB, 1).output)
        .output_data(kit::amount(1).pack())
        .build();
    // The pool rejects the unexempted foreign LP before settlement, even though
    // the current pool's own LP is permitted to mint during liquidity changes.
    f.reject(tx, Error::Token);
}

#[test]
fn pool_owner_mode_still_requires_exact_lp_mint_and_redemption_burn() {
    for mode in 0..3 {
        let mut f = Fixture::new(false, true);
        f.start();
        let tx = if mode == 0 {
            let tx = f.update(f.swap(1, false), 1);
            let change = tx
                .outputs()
                .get(5)
                .unwrap()
                .as_builder()
                .capacity(3399 * CKB)
                .build();
            let tx = kit::replace_output(tx, 5, change);
            // State Type is present, so xUDT owner mode permits this mint.
            // The pool must reject it because a swap mints zero LP.
            tx.as_advanced_builder()
                .output(Cell::token(&f.wallets[1], &f.args.lp_type, 200 * CKB, 1).output)
                .output_data(kit::amount(1).pack())
                .build()
        } else {
            let shares = 200_000;
            let tx = f.update(f.remove(0, shares), 1);
            let expected = f.state.total_shares - MINIMUM_LIQUIDITY - shares;
            kit::replace_data(
                tx,
                5,
                kit::amount(if mode == 1 {
                    expected + 1
                } else {
                    expected - 1
                }),
            )
        };
        f.reject(tx, Error::Token);
    }
}

#[test]
fn lp_mint_exemption_matches_full_type_including_owner_flags_and_hash_type() {
    for mode in 0..4 {
        let mut f = Fixture::new(false, true);
        let tx = f.creation();
        let mut args = f.args.lp_type.args().raw_data().to_vec();
        match mode {
            0 => args[32..].copy_from_slice(&0u32.to_le_bytes()),
            1 => args[32..].copy_from_slice(&0xa000_0000u32.to_le_bytes()),
            2 => args[..32].copy_from_slice(&f.hash(0)),
            _ => {}
        }
        let mut fake = f
            .args
            .lp_type
            .clone()
            .as_builder()
            .args(args.pack())
            .build();
        if mode == 3 {
            fake = fake.as_builder().hash_type(ScriptHashType::Data2).build();
        }
        let output = tx
            .outputs()
            .get(3)
            .unwrap()
            .as_builder()
            .type_(Some(fake).pack())
            .build();
        let tx = kit::replace_output(tx, 3, output);
        f.reject(tx, Error::Token);
    }
}

#[test]
fn pool_rejects_state_and_lp_types_used_as_input_locks() {
    for lp_lock in [false, true] {
        let mut f = Fixture::new(false, true);
        f.start();
        let tx = f.update(f.swap(1, false), 1);
        let lock = if lp_lock {
            &f.args.lp_type
        } else {
            &f.args.state_type
        };
        let cell = Cell::capacity(lock, 1000 * CKB);
        // Valid amount bytes let the xUDT branch run if used as a Lock; the pool
        // independently rejects the misuse rather than relying on malformed data.
        let point = f.context.create_cell(cell.output, kit::amount(0));
        let tx = tx
            .as_advanced_builder()
            .input(input(point))
            .output(Cell::capacity(&f.wallets[1], 999 * CKB).output)
            .output_data(Bytes::new().pack())
            .build();
        f.reject(tx, Error::Authorization);
    }
}
