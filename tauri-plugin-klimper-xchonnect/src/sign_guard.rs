// SPDX-License-Identifier: AGPL-3.0-or-later
//! Refuse spends whose conditions the signature does not fix (clapandpay TASK-141).
//!
//! A standard coin (`p2_delegated_puzzle_or_hidden_puzzle`) is spent with a *delegated
//! puzzle*, and its key signs that delegated puzzle's tree hash: the signature covers the
//! program, not what the program returns. Only a quoted delegated puzzle, `(q . conditions)`,
//! returns the same conditions whatever the rest of the solution says, so only then does
//! the signature fix what the spend does. The same holds for the hidden-puzzle path, which
//! is not signed for at all.
//!
//! So before anything the wallet would sign is shown or signed, every spend is checked:
//! each standard puzzle the wallet holds the key for — the coin's own puzzle, or the inner
//! puzzle of a CAT, NFT, DID, option or another layer the driver knows — must be spent with
//! a quoted delegated puzzle and without the hidden puzzle. A standard puzzle with one of
//! the wallet's keys anywhere else in a spend, in a shape this module cannot follow, is
//! refused as well. The answer is the standard refusal, `4001` with the reason
//! `unsigned_conditions`, and the user is never asked.
//!
//! Only the keys the request may sign with are looked at (for a website's own spends, the
//! keys it was shown): a spend for any other key is never signed, and a refusal here must
//! not tell a website which other keys the wallet holds.
//!
//! One more check covers `AGG_SIG_UNSAFE`, which a website may use only once the user has
//! allowed it: its signature is over the bare message, with no coin or network data, so a
//! message chosen to look like what another signature covers would get the wallet to make
//! that other signature. The spends are run as wallet-kit runs them, and an
//! `AGG_SIG_UNSAFE` whose message has the shape of another kind of signature — it ends
//! with any of the network's `AGG_SIG` additional data, as every coin-bound signature
//! does (the chain itself refuses such a message), or it has the size of a signed message
//! (`signMessage`) or starts with its framing — is refused with `4001` and the reason
//! `unsafe_message`, whoever's key it names.

use std::collections::{HashMap, HashSet};

use chia_bls::PublicKey;
use chia_protocol::{Bytes32, CoinSpend};
use chia_puzzle_types::did::DidSolution;
use chia_puzzle_types::standard::StandardSolution;
use chia_wallet_sdk::clvm_traits::{FromClvm, ToClvm};
use chia_wallet_sdk::clvmr::serde::node_from_bytes;
use chia_wallet_sdk::clvmr::{Allocator, NodePtr, SExp};
use chia_wallet_sdk::driver::{
    AugmentedConditionLayer, CatLayer, DidLayer, Layer, NftOwnershipLayer, NftStateLayer,
    OptionContractLayer, P2OneOfManyLayer, Puzzle, RevocationLayer, SingletonLayer, StandardLayer,
};
use chia_wallet_sdk::prelude::Condition;
use chia_wallet_sdk::puzzles::P2_DELEGATED_PUZZLE_OR_HIDDEN_PUZZLE;
use chia_wallet_sdk::signer::AggSigConstants;
use chia_wallet_sdk::types::{MAINNET_CONSTANTS, TESTNET11_CONSTANTS};
use serde_json::Value;
use xchonnect_core::rpc::{canonical_method, codes};
use xchonnect_wallet_kit::{self as kit, RpcError};

/// The reason a refusal names.
pub const REASON: &str = "unsigned_conditions";

/// The reason an `AGG_SIG_UNSAFE` refusal names.
pub const UNSAFE_REASON: &str = "unsafe_message";

/// The text `signMessage` frames a message with before it is hashed and signed.
const SIGNED_MESSAGE_PREFIX: &[u8] = b"Chia Signed Message";

/// How many layers are followed down to a standard puzzle. Real assets have at most four
/// (singleton, NFT state, NFT ownership, standard).
const MAX_DEPTH: usize = 8;

/// The refusal: `4001`, reason [`REASON`].
pub fn refusal() -> RpcError {
    RpcError {
        code: codes::UNAUTHORIZED,
        message: "the wallet does not sign a spend whose conditions its signature does not fix"
            .to_owned(),
        data: Some(serde_json::json!({ "reason": REASON }).to_string()),
    }
}

/// The refusal of an `AGG_SIG_UNSAFE` message: `4001`, reason [`UNSAFE_REASON`].
pub fn unsafe_refusal() -> RpcError {
    RpcError {
        code: codes::UNAUTHORIZED,
        message: "the wallet does not sign an AGG_SIG_UNSAFE message of this shape".to_owned(),
        data: Some(serde_json::json!({ "reason": UNSAFE_REASON }).to_string()),
    }
}

/// For a request that carries spends a website built (`signCoinSpends`,
/// `xchonnect_submitCoinSpends`): check them with [`check_spends`]. Other methods, and
/// params that do not parse (wallet-kit refuses those itself), pass.
// The key set is wallet-kit's `RequestContext::keys`, with the default hasher.
#[allow(clippy::implicit_hasher)]
pub fn check_request(
    method: &str,
    params: &str,
    keys: &HashSet<PublicKey>,
    network: &kit::Network,
) -> Result<(), RpcError> {
    if !matches!(
        canonical_method(method),
        "signCoinSpends" | kit::SUBMIT_COIN_SPENDS
    ) {
        return Ok(());
    }
    let Ok(value) = serde_json::from_str::<Value>(params) else {
        return Ok(());
    };
    let Ok(spends) = kit::parse_coin_spends(value.get("coinSpends").unwrap_or(&Value::Null)) else {
        return Ok(());
    };
    check_spends(&spends, keys, network)
}

/// Refuse if any spend would be signed with one of `keys` without the signature fixing its
/// conditions, or if any spend asks for an `AGG_SIG_UNSAFE` signature of a message that
/// could stand in for another kind of signature on `network`.
// The key set is wallet-kit's `RequestContext::keys`, with the default hasher.
#[allow(clippy::implicit_hasher)]
pub fn check_spends(
    spends: &[CoinSpend],
    keys: &HashSet<PublicKey>,
    network: &kit::Network,
) -> Result<(), RpcError> {
    if keys.is_empty() {
        return Ok(());
    }
    if !spends.iter().all(|spend| fixed(spend, keys)) {
        return Err(refusal());
    }
    check_unsafe_messages(spends, network)
}

/// Run the spends as wallet-kit does and refuse an `AGG_SIG_UNSAFE` message of the shape
/// [`unsafe_message_refused`] describes. Spends that do not run are left to wallet-kit,
/// which refuses them itself.
fn check_unsafe_messages(spends: &[CoinSpend], network: &kit::Network) -> Result<(), RpcError> {
    let mut allocator = Allocator::new();
    let Ok((executed, _)) = kit::simulate::execute(
        &mut allocator,
        spends,
        &kit::Ownership::default(),
        kit::simulate::DEFAULT_MAX_COST,
    ) else {
        return Ok(());
    };
    let suffixes = additional_data(network);
    let refused = executed
        .iter()
        .flat_map(|spend| &spend.conditions)
        .any(|condition| match condition {
            Condition::AggSigUnsafe(agg) => unsafe_message_refused(agg.message.as_ref(), &suffixes),
            _ => false,
        });
    if refused {
        Err(unsafe_refusal())
    } else {
        Ok(())
    }
}

/// Every `AGG_SIG` additional data value a coin-bound signature's message ends with: those
/// of `network`, and of mainnet and testnet11 whatever the network, so a signature made on
/// one can never be one for the other.
fn additional_data(network: &kit::Network) -> Vec<Bytes32> {
    let mut genesis = vec![
        MAINNET_CONSTANTS.agg_sig_me_additional_data,
        TESTNET11_CONSTANTS.agg_sig_me_additional_data,
    ];
    if let kit::Network::Custom { agg_sig_me, .. } = network {
        genesis.push(*agg_sig_me);
    }
    genesis
        .into_iter()
        .flat_map(|agg_sig_me| {
            let constants = AggSigConstants::new(agg_sig_me);
            [
                constants.me(),
                constants.parent(),
                constants.puzzle(),
                constants.amount(),
                constants.puzzle_amount(),
                constants.parent_amount(),
                constants.parent_puzzle(),
            ]
        })
        .collect()
}

/// An `AGG_SIG_UNSAFE` message that could be what another signature covers: one that ends
/// with an `AGG_SIG` additional data value (every `AGG_SIG_ME`, `AGG_SIG_PARENT`, …
/// message does), one of 32 bytes (the hash `signMessage` signs) or one that starts with
/// `signMessage`'s framing.
fn unsafe_message_refused(message: &[u8], suffixes: &[Bytes32]) -> bool {
    message.len() == 32
        || message.starts_with(SIGNED_MESSAGE_PREFIX)
        || suffixes
            .iter()
            .any(|suffix| message.ends_with(suffix.as_ref()))
}

/// Whether every standard puzzle with one of `keys` in this spend is spent with a quoted
/// delegated puzzle.
fn fixed(spend: &CoinSpend, keys: &HashSet<PublicKey>) -> bool {
    let mut allocator = Allocator::new();
    let (Ok(puzzle), Ok(solution)) = (
        spend.puzzle_reveal.to_clvm(&mut allocator),
        spend.solution.to_clvm(&mut allocator),
    ) else {
        // Not CLVM: wallet-kit cannot run it either, and refuses it.
        return true;
    };
    let mut checked = HashSet::new();
    let puzzle_parsed = Puzzle::parse(&allocator, puzzle);
    if !follow(&allocator, puzzle_parsed, solution, keys, &mut checked, 0) {
        return false;
    }
    // Any of the wallet's standard puzzles in the spend that the walk did not reach is in a
    // shape it cannot read, so nothing is known about what its signature would fix.
    let mut scan = Scan::new(&allocator, keys);
    scan.run(puzzle);
    scan.run(solution);
    scan.found.iter().all(|ptr| checked.contains(ptr))
}

/// A delegated puzzle `(q . anything)`: it returns `anything` whatever its solution.
fn is_quote(allocator: &Allocator, program: NodePtr) -> bool {
    match allocator.sexp(program) {
        SExp::Pair(first, _) => {
            matches!(allocator.sexp(first), SExp::Atom) && allocator.atom(first).as_ref() == [1]
        }
        SExp::Atom => false,
    }
}

/// Peel known layers down to the standard puzzles in `puzzle`, run with `solution`, and
/// check each one with one of `keys`. Returns `false` for a violation; records each checked
/// standard puzzle in `checked`. A layer whose solution does not parse is not followed: the
/// scan in [`fixed`] then finds what was not checked.
fn follow(
    allocator: &Allocator,
    puzzle: Puzzle,
    solution: NodePtr,
    keys: &HashSet<PublicKey>,
    checked: &mut HashSet<NodePtr>,
    depth: usize,
) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    let inner = |puzzle: Puzzle, solution: NodePtr, checked: &mut HashSet<NodePtr>| {
        follow(allocator, puzzle, solution, keys, checked, depth + 1)
    };

    if let Ok(Some(standard)) = StandardLayer::parse_puzzle(allocator, puzzle) {
        if !keys.contains(&standard.synthetic_key) {
            return true;
        }
        let Ok(parsed) = StandardSolution::<NodePtr, NodePtr>::from_clvm(allocator, solution)
        else {
            return false;
        };
        // The hidden puzzle is not signed for; a quoted delegated puzzle is the only one
        // that returns the same conditions for every solution.
        if parsed.original_public_key.is_some() || !is_quote(allocator, parsed.delegated_puzzle) {
            return false;
        }
        checked.insert(puzzle.ptr());
        return true;
    }

    // Each wrapping layer: its inner puzzle and the inner puzzle's solution.
    macro_rules! wrapped {
        ($layer:ty, |$sol:ident| $inner_solution:expr) => {
            if let Ok(Some(layer)) = <$layer>::parse_puzzle(allocator, puzzle) {
                return match <$layer>::parse_solution(allocator, solution) {
                    Ok($sol) => inner(layer.inner_puzzle, $inner_solution, checked),
                    Err(_) => true,
                };
            }
        };
    }
    wrapped!(CatLayer<Puzzle>, |s| s.inner_puzzle_solution);
    wrapped!(SingletonLayer<Puzzle>, |s| s.inner_solution);
    wrapped!(NftStateLayer<NodePtr, Puzzle>, |s| s.inner_solution);
    wrapped!(NftOwnershipLayer<Puzzle, Puzzle>, |s| s.inner_solution);
    wrapped!(OptionContractLayer<Puzzle>, |s| s.inner_solution);
    wrapped!(AugmentedConditionLayer<NodePtr, Puzzle>, |s| s.inner_solution);
    if let Ok(Some(layer)) = DidLayer::<NodePtr, Puzzle>::parse_puzzle(allocator, puzzle) {
        return match DidLayer::<NodePtr, Puzzle>::parse_solution(allocator, solution) {
            Ok(DidSolution::Spend(inner_solution)) => {
                inner(layer.inner_puzzle, inner_solution, checked)
            }
            // A recovery does not run the inner puzzle.
            Ok(DidSolution::Recover(_)) | Err(_) => true,
        };
    }
    // Layers that reveal their inner puzzle in the solution.
    if matches!(
        RevocationLayer::parse_puzzle(allocator, puzzle),
        Ok(Some(_))
    ) {
        return match RevocationLayer::parse_solution(allocator, solution) {
            Ok(s) => inner(Puzzle::parse(allocator, s.puzzle), s.solution, checked),
            Err(_) => true,
        };
    }
    if matches!(
        P2OneOfManyLayer::parse_puzzle(allocator, puzzle),
        Ok(Some(_))
    ) {
        return match P2OneOfManyLayer::parse_solution(allocator, solution) {
            Ok(s) => inner(Puzzle::parse(allocator, s.puzzle), s.solution, checked),
            Err(_) => true,
        };
    }
    true
}

/// Every standard puzzle with one of the wallet's keys in a CLVM tree, wherever it is.
struct Scan<'a> {
    allocator: &'a Allocator,
    /// The keys, serialized: a candidate's key is compared as bytes, never parsed (a
    /// point decompression per candidate would be the slowest part of the scan).
    keys: HashSet<[u8; 48]>,
    visited: HashSet<NodePtr>,
    /// The standard puzzle, to compare a candidate's program with. Compared node by node,
    /// not hashed: a comparison stops at the first difference and never looks at more of a
    /// program than the standard puzzle has, so nested candidates cost no more than their
    /// own size.
    reference: Allocator,
    reference_root: NodePtr,
    /// Whether a curried program is the standard puzzle, per program node.
    standard_mod: HashMap<NodePtr, bool>,
    found: Vec<NodePtr>,
}

impl<'a> Scan<'a> {
    fn new(allocator: &'a Allocator, keys: &'a HashSet<PublicKey>) -> Self {
        let mut reference = Allocator::new();
        // The puzzle's own bytes: they always parse.
        let reference_root = node_from_bytes(&mut reference, &P2_DELEGATED_PUZZLE_OR_HIDDEN_PUZZLE)
            .unwrap_or(NodePtr::NIL);
        Scan {
            allocator,
            keys: keys.iter().map(PublicKey::to_bytes).collect(),
            visited: HashSet::new(),
            reference,
            reference_root,
            standard_mod: HashMap::new(),
            found: Vec::new(),
        }
    }

    fn run(&mut self, root: NodePtr) {
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            let SExp::Pair(first, rest) = self.allocator.sexp(node) else {
                continue;
            };
            if !self.visited.insert(node) {
                continue;
            }
            if self.is_ours(node) {
                self.found.push(node);
            }
            stack.push(first);
            stack.push(rest);
        }
    }

    /// `(a (q . MOD) (c (q . KEY) 1))` with `MOD` the standard puzzle and `KEY` one of ours.
    fn is_ours(&mut self, node: NodePtr) -> bool {
        let a = self.allocator;
        let pair = |n: NodePtr| match a.sexp(n) {
            SExp::Pair(first, rest) => Some((first, rest)),
            SExp::Atom => None,
        };
        let atom_is = |n: NodePtr, bytes: &[u8]| {
            matches!(a.sexp(n), SExp::Atom) && a.atom(n).as_ref() == bytes
        };
        let shape = || {
            let (op, rest) = pair(node)?;
            let (quoted_mod, rest) = pair(rest)?;
            let (args, end) = pair(rest)?;
            let (q, program) = pair(quoted_mod)?;
            let (cons, rest) = pair(args)?;
            let (quoted_key, rest) = pair(rest)?;
            let (env, args_end) = pair(rest)?;
            let (q2, key) = pair(quoted_key)?;
            let well_formed = atom_is(op, &[2])
                && atom_is(end, &[])
                && atom_is(q, &[1])
                && atom_is(cons, &[4])
                && atom_is(q2, &[1])
                && atom_is(env, &[1])
                && atom_is(args_end, &[]);
            well_formed.then_some((program, key))
        };
        let Some((program, key)) = shape() else {
            return false;
        };
        if !matches!(a.sexp(key), SExp::Atom) {
            return false;
        }
        let Ok(bytes) = <[u8; 48]>::try_from(a.atom(key).as_ref()) else {
            return false;
        };
        if !self.keys.contains(&bytes) {
            return false;
        }
        if let Some(&known) = self.standard_mod.get(&program) {
            return known;
        }
        let standard = self.is_standard_mod(program);
        self.standard_mod.insert(program, standard);
        standard
    }

    /// Whether `program` is the standard puzzle, node by node.
    fn is_standard_mod(&self, program: NodePtr) -> bool {
        let (a, r) = (self.allocator, &self.reference);
        let mut stack = vec![(program, self.reference_root)];
        while let Some((node, expected)) = stack.pop() {
            match (a.sexp(node), r.sexp(expected)) {
                (SExp::Atom, SExp::Atom) => {
                    if a.atom(node).as_ref() != r.atom(expected).as_ref() {
                        return false;
                    }
                }
                (SExp::Pair(first, rest), SExp::Pair(expected_first, expected_rest)) => {
                    stack.push((rest, expected_rest));
                    stack.push((first, expected_first));
                }
                _ => return false,
            }
        }
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chia_protocol::{Bytes32, Coin, Program};
    use chia_puzzle_types::Memos;
    use chia_puzzle_types::cat::CatSolution;
    use chia_wallet_sdk::driver::{
        Cat, CatInfo, CatSpend, Spend, SpendContext, SpendWithConditions,
    };
    use chia_wallet_sdk::prelude::Conditions;
    use chia_wallet_sdk::test::BlsPair;

    fn keys(who: &BlsPair) -> HashSet<PublicKey> {
        [who.pk].into_iter().collect()
    }

    /// [`super::check_spends`] on testnet11.
    fn check_spends(spends: &[CoinSpend], keys: &HashSet<PublicKey>) -> Result<(), RpcError> {
        super::check_spends(spends, keys, &kit::Network::Testnet11)
    }

    /// A standard spend of `who`'s coin with `delegated` as the delegated puzzle and
    /// `solution` as its solution.
    fn standard_with(
        who: &BlsPair,
        delegated: impl Fn(&mut SpendContext) -> NodePtr,
        solution: impl Fn(&mut SpendContext) -> NodePtr,
    ) -> CoinSpend {
        let mut ctx = SpendContext::new();
        let coin = Coin::new(Bytes32::new([1; 32]), who.puzzle_hash, 1_000);
        let delegated = delegated(&mut ctx);
        let solution = solution(&mut ctx);
        StandardLayer::new(who.pk)
            .delegated_inner_spend(&mut ctx, Spend::new(delegated, solution))
            .and_then(|spend| ctx.spend(coin, spend))
            .unwrap();
        ctx.take().remove(0)
    }

    fn pay(ctx: &mut SpendContext, to: Bytes32) -> NodePtr {
        ctx.alloc(&Conditions::new().create_coin(to, 1_000, Memos::None))
            .unwrap()
    }

    fn program(ctx: &mut SpendContext, bytes: &[u8]) -> NodePtr {
        ctx.alloc(&Program::from(bytes.to_vec())).unwrap()
    }

    #[test]
    fn a_quoted_delegated_puzzle_passes() {
        let who = BlsPair::new(1);
        let mut ctx = SpendContext::new();
        let coin = Coin::new(Bytes32::new([1; 32]), who.puzzle_hash, 1_000);
        StandardLayer::new(who.pk)
            .spend(
                &mut ctx,
                coin,
                Conditions::new().create_coin(who.puzzle_hash, 1_000, Memos::None),
            )
            .unwrap();
        assert!(check_spends(&ctx.take(), &keys(&who)).is_ok());
    }

    /// The delegated puzzle `1` returns its solution: the conditions are not signed.
    #[test]
    fn a_delegated_puzzle_that_returns_its_solution_is_refused() {
        let who = BlsPair::new(1);
        let spend = standard_with(
            &who,
            |ctx| program(ctx, &[0x01]),
            |ctx| pay(ctx, who.puzzle_hash),
        );
        let error = check_spends(&[spend], &keys(&who)).unwrap_err();
        assert_eq!(error.code, codes::UNAUTHORIZED);
        assert_eq!(
            error.data.as_deref(),
            Some(r#"{"reason":"unsigned_conditions"}"#)
        );
    }

    /// Other programs whose output depends on the solution: part of it, `(f 1)`, and an
    /// apply of a program from it, `(a 2 3)`.
    #[test]
    fn other_programs_that_read_their_solution_are_refused() {
        let who = BlsPair::new(1);
        // (f 1)
        let first = standard_with(
            &who,
            |ctx| program(ctx, &[0xff, 0x05, 0xff, 0x01, 0x80]),
            |ctx| {
                let conditions = pay(ctx, who.puzzle_hash);
                ctx.alloc(&(conditions, ())).unwrap()
            },
        );
        // (a 2 3)
        let apply = standard_with(
            &who,
            |ctx| program(ctx, &[0xff, 0x02, 0xff, 0x02, 0xff, 0x03, 0x80]),
            |ctx| {
                let conditions = pay(ctx, who.puzzle_hash);
                let quoted = ctx.alloc(&(1, conditions)).unwrap();
                ctx.alloc(&(quoted, ())).unwrap()
            },
        );
        for spend in [first, apply] {
            assert_eq!(
                check_spends(&[spend], &keys(&who))
                    .unwrap_err()
                    .data
                    .as_deref(),
                Some(r#"{"reason":"unsigned_conditions"}"#)
            );
        }
    }

    /// The hidden-puzzle path is refused whatever the hidden puzzle is.
    #[test]
    fn the_hidden_puzzle_path_is_refused() {
        let who = BlsPair::new(1);
        let mut ctx = SpendContext::new();
        let coin = Coin::new(Bytes32::new([1; 32]), who.puzzle_hash, 1_000);
        let conditions = pay(&mut ctx, who.puzzle_hash);
        let quoted = ctx.alloc(&(1, conditions)).unwrap();
        let solution = ctx
            .alloc(&StandardSolution {
                original_public_key: Some(BlsPair::new(2).pk),
                delegated_puzzle: quoted,
                solution: NodePtr::NIL,
            })
            .unwrap();
        let puzzle = StandardLayer::new(who.pk)
            .construct_puzzle(&mut ctx)
            .unwrap();
        ctx.spend(coin, Spend::new(puzzle, solution)).unwrap();
        assert!(check_spends(&ctx.take(), &keys(&who)).is_err());
    }

    /// Only the keys the request may sign with count: another key's spend is not the
    /// wallet's to sign, and is left alone (wallet-kit refuses to sign it).
    #[test]
    fn another_keys_spend_is_left_alone() {
        let who = BlsPair::new(1);
        let spend = standard_with(
            &who,
            |ctx| program(ctx, &[0x01]),
            |ctx| pay(ctx, who.puzzle_hash),
        );
        assert!(check_spends(&[spend], &keys(&BlsPair::new(2))).is_ok());
    }

    fn cat_spend(who: &BlsPair, inner: Spend, ctx: &mut SpendContext) -> CoinSpend {
        let info = CatInfo::new(Bytes32::new([7; 32]), None, who.puzzle_hash);
        let cat = Cat::new(
            Coin::new(Bytes32::new([2; 32]), info.puzzle_hash().into(), 1_000),
            None,
            info,
        );
        Cat::spend_all(ctx, &[CatSpend::new(cat, inner)]).unwrap();
        ctx.take().remove(0)
    }

    /// A CAT whose inner standard spend reads its solution is refused like a plain one; the
    /// quoted one passes.
    #[test]
    fn a_cat_with_an_unsigned_inner_spend_is_refused() {
        let who = BlsPair::new(1);
        let mut ctx = SpendContext::new();
        let one = program(&mut ctx, &[0x01]);
        let conditions = pay(&mut ctx, who.puzzle_hash);
        let inner = StandardLayer::new(who.pk)
            .delegated_inner_spend(&mut ctx, Spend::new(one, conditions))
            .unwrap();
        let bad = cat_spend(&who, inner, &mut ctx);
        assert!(check_spends(&[bad], &keys(&who)).is_err());

        let mut ctx = SpendContext::new();
        let inner = StandardLayer::new(who.pk)
            .spend_with_conditions(
                &mut ctx,
                Conditions::new().create_coin(who.puzzle_hash, 1_000, Memos::None),
            )
            .unwrap();
        let good = cat_spend(&who, inner, &mut ctx);
        assert!(check_spends(&[good], &keys(&who)).is_ok());
    }

    /// The wallet's standard puzzle in a place the walk does not follow is refused: here, a
    /// CAT solution that does not parse, so the inner spend cannot be read.
    #[test]
    fn a_standard_puzzle_in_an_unreadable_place_is_refused() {
        let who = BlsPair::new(1);
        let mut ctx = SpendContext::new();
        let inner = StandardLayer::new(who.pk)
            .spend_with_conditions(
                &mut ctx,
                Conditions::new().create_coin(who.puzzle_hash, 1_000, Memos::None),
            )
            .unwrap();
        let mut spend = cat_spend(&who, inner, &mut ctx);
        let mut allocator = Allocator::new();
        let solution = spend.solution.to_clvm(&mut allocator).unwrap();
        let cat = CatSolution::<NodePtr>::from_clvm(&allocator, solution).unwrap();
        // Truncated: the CAT solution's fields after the inner solution are gone.
        let truncated = allocator
            .new_pair(cat.inner_puzzle_solution, NodePtr::NIL)
            .unwrap();
        spend.solution = Program::from_clvm(&allocator, truncated).unwrap();
        assert!(check_spends(&[spend], &keys(&who)).is_err());
    }

    #[test]
    fn only_methods_with_website_built_spends_are_checked() {
        let who = BlsPair::new(1);
        let spend = standard_with(
            &who,
            |ctx| program(ctx, &[0x01]),
            |ctx| pay(ctx, who.puzzle_hash),
        );
        let params = serde_json::json!({
            "coinSpends": [{
                "coin": {
                    "parent_coin_info": format!("0x{}", hex::encode(spend.coin.parent_coin_info)),
                    "puzzle_hash": format!("0x{}", hex::encode(spend.coin.puzzle_hash)),
                    "amount": spend.coin.amount,
                },
                "puzzle_reveal": format!("0x{}", hex::encode(spend.puzzle_reveal.as_ref())),
                "solution": format!("0x{}", hex::encode(spend.solution.as_ref())),
            }]
        })
        .to_string();
        let keys = keys(&who);
        let network = kit::Network::Testnet11;
        let alias = format!("chip0002_{}", kit::SUBMIT_COIN_SPENDS);
        for method in [
            "signCoinSpends",
            "chip0002_signCoinSpends",
            kit::SUBMIT_COIN_SPENDS,
            &alias,
        ] {
            assert!(
                check_request(method, &params, &keys, &network).is_err(),
                "{method}"
            );
        }
        assert!(check_request("signMessage", &params, &keys, &network).is_ok());
        assert!(check_request("signCoinSpends", "not json", &keys, &network).is_ok());
    }

    /// A standard spend of `who`'s coin, with quoted conditions, that asks for an
    /// `AGG_SIG_UNSAFE` signature of `message` by `signer`.
    fn unsafe_spend(who: &BlsPair, signer: PublicKey, message: Vec<u8>) -> CoinSpend {
        let mut ctx = SpendContext::new();
        let coin = Coin::new(Bytes32::new([1; 32]), who.puzzle_hash, 1_000);
        StandardLayer::new(who.pk)
            .spend(
                &mut ctx,
                coin,
                Conditions::new()
                    .create_coin(who.puzzle_hash, 1_000, Memos::None)
                    .agg_sig_unsafe(signer, message.into()),
            )
            .unwrap();
        ctx.take().remove(0)
    }

    fn unsafe_reason(result: Result<(), RpcError>) -> Option<String> {
        result.err().and_then(|error| {
            assert_eq!(error.code, codes::UNAUTHORIZED);
            error.data
        })
    }

    const UNSAFE: &str = r#"{"reason":"unsafe_message"}"#;

    /// Each `AGG_SIG` additional data value of mainnet and testnet11, as the chain derives
    /// them, independently of the code under test.
    fn every_suffix() -> Vec<(&'static str, Bytes32)> {
        use chia_wallet_sdk::chia::sha2::Sha256;
        let mut out = Vec::new();
        for (name, me) in [
            ("mainnet", MAINNET_CONSTANTS.agg_sig_me_additional_data),
            ("testnet11", TESTNET11_CONSTANTS.agg_sig_me_additional_data),
        ] {
            out.push((name, me));
            // AGG_SIG_PARENT … AGG_SIG_PARENT_PUZZLE: sha256(agg_sig_me + opcode).
            for opcode in 43u8..=48 {
                let mut hasher = Sha256::new();
                hasher.update(me);
                hasher.update([opcode]);
                out.push((name, Bytes32::new(hasher.finalize())));
            }
        }
        out
    }

    /// The derivation agrees with the consensus constants of both networks.
    #[test]
    fn the_suffixes_are_the_consensus_constants() {
        for constants in [&MAINNET_CONSTANTS, &TESTNET11_CONSTANTS] {
            let expected = [
                constants.agg_sig_me_additional_data,
                constants.agg_sig_parent_additional_data,
                constants.agg_sig_puzzle_additional_data,
                constants.agg_sig_amount_additional_data,
                constants.agg_sig_puzzle_amount_additional_data,
                constants.agg_sig_parent_amount_additional_data,
                constants.agg_sig_parent_puzzle_additional_data,
            ];
            let ours = additional_data(&kit::Network::Testnet11);
            for value in expected {
                assert!(ours.contains(&value));
            }
        }
        assert_eq!(additional_data(&kit::Network::Mainnet).len(), 14);
    }

    /// An `AGG_SIG_UNSAFE` message ending with any network's additional data is refused,
    /// on either network, before anything else: what a coin-bound signature's message
    /// ends with.
    #[test]
    fn an_unsafe_message_with_a_coin_bound_suffix_is_refused() {
        let who = BlsPair::new(1);
        let suffixes = every_suffix();
        assert_eq!(suffixes.len(), 14);
        for network in [kit::Network::Mainnet, kit::Network::Testnet11] {
            for (name, suffix) in &suffixes {
                // A coin id and an amount in front, as a real AGG_SIG_ME message has.
                let mut message = vec![7u8; 40];
                message.extend_from_slice(suffix.as_ref());
                let spend = unsafe_spend(&who, who.pk, message);
                assert_eq!(
                    unsafe_reason(super::check_spends(&[spend], &keys(&who), &network)).as_deref(),
                    Some(UNSAFE),
                    "{name} suffix {suffix} on {network:?}"
                );
            }
        }
        // And on a custom network, its own.
        let custom = kit::Network::Custom {
            chain_id: "simulator0".to_owned(),
            genesis_challenge: Bytes32::new([3; 32]),
            agg_sig_me: Bytes32::new([4; 32]),
        };
        let mut message = b"anything".to_vec();
        message.extend_from_slice(&AggSigConstants::new(Bytes32::new([4; 32])).amount());
        let spend = unsafe_spend(&who, who.pk, message);
        assert_eq!(
            unsafe_reason(super::check_spends(&[spend], &keys(&who), &custom)).as_deref(),
            Some(UNSAFE)
        );
    }

    /// The message `signMessage` signs is a 32-byte hash of the framed message: an unsafe
    /// message of that size, or one starting with the framing, is refused.
    #[test]
    fn an_unsafe_message_that_could_be_a_signed_message_is_refused() {
        let who = BlsPair::new(1);
        let hashed = kit::signed_message_hash(b"I agree").to_vec();
        let framed = b"Chia Signed Message: I agree".to_vec();
        for message in [hashed, framed, vec![0; 32]] {
            let spend = unsafe_spend(&who, who.pk, message);
            assert_eq!(
                unsafe_reason(check_spends(&[spend], &keys(&who))).as_deref(),
                Some(UNSAFE)
            );
        }
    }

    /// Whoever's key it names: a refusal must not depend on which keys the wallet holds.
    #[test]
    fn the_unsafe_check_does_not_depend_on_the_key() {
        let who = BlsPair::new(1);
        let other = BlsPair::new(2);
        let spend = unsafe_spend(&who, other.pk, vec![9; 32]);
        assert_eq!(
            unsafe_reason(check_spends(&[spend], &keys(&who))).as_deref(),
            Some(UNSAFE)
        );
    }

    /// Other unsafe messages pass the guard (wallet-kit then asks, if the user allowed
    /// `AGG_SIG_UNSAFE` for the website).
    #[test]
    fn an_honest_unsafe_message_passes() {
        let who = BlsPair::new(1);
        let mut long = vec![5u8; 64];
        long.extend_from_slice(b"end");
        for message in [
            b"hello".to_vec(),
            vec![1; 31],
            vec![1; 33],
            long,
            b"Chia: a message".to_vec(),
        ] {
            let spend = unsafe_spend(&who, who.pk, message.clone());
            assert!(
                check_spends(&[spend], &keys(&who)).is_ok(),
                "{}",
                hex::encode(&message)
            );
        }
    }

    /// Wallet-built (`(a (q . MOD) (c (q . KEY) 1))`) shapes nested inside each other, each
    /// with one of the wallet's keys and each a candidate: hashing each candidate's program
    /// anew would cost the square of the depth. The memoized scan stays well under 100 ms.
    #[test]
    fn nested_candidates_are_scanned_in_linear_time() {
        let who = BlsPair::new(1);
        let mut allocator = Allocator::new();
        let key = allocator.new_atom(&who.pk.to_bytes()).unwrap();
        let atom = |a: &mut Allocator, byte: u8| a.new_small_number(u32::from(byte)).unwrap();
        let mut program = NodePtr::NIL;
        for _ in 0..3_000 {
            // (a (q . program) (c (q . KEY) 1))
            let (two, one, four) = (
                atom(&mut allocator, 2),
                atom(&mut allocator, 1),
                atom(&mut allocator, 4),
            );
            let quoted_key = allocator.new_pair(one, key).unwrap();
            let env_tail = allocator.new_pair(one, NodePtr::NIL).unwrap();
            let args = allocator.new_pair(quoted_key, env_tail).unwrap();
            let args = allocator.new_pair(four, args).unwrap();
            let quoted = allocator.new_pair(one, program).unwrap();
            let rest = allocator.new_pair(args, NodePtr::NIL).unwrap();
            let rest = allocator.new_pair(quoted, rest).unwrap();
            program = allocator.new_pair(two, rest).unwrap();
        }
        let keys = keys(&who);
        let started = std::time::Instant::now();
        let mut scan = Scan::new(&allocator, &keys);
        scan.run(program);
        let elapsed = started.elapsed();
        assert!(scan.found.is_empty(), "none of them is the standard puzzle");
        assert!(
            elapsed < std::time::Duration::from_millis(100),
            "the scan took {elapsed:?}"
        );

        // Through the whole guard, as a spend's solution.
        let solution = Program::from_clvm(&allocator, program).unwrap();
        let spend = CoinSpend::new(
            Coin::new(Bytes32::new([1; 32]), Bytes32::new([2; 32]), 1),
            Program::from(vec![0xff, 0x01, 0x80]),
            solution,
        );
        let started = std::time::Instant::now();
        assert!(check_spends(&[spend], &keys).is_ok());
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(100),
            "the guard took {elapsed:?}"
        );
    }
}
