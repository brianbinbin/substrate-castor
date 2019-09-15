use support::{decl_module, decl_storage, decl_event, StorageValue, StorageMap, dispatch::Result, Parameter, ensure};
use sr_primitives::traits::{ Member, SimpleArithmetic, Bounded, CheckedAdd };
use system::ensure_signed;
use codec::{Encode, Decode};
use rstd::result;
use crate::ge;
use crate::node;
use support::traits::{Currency};


/// The module's configuration trait.
pub trait Trait: system::Trait + ge::Trait + timestamp::Trait + node::Trait {
	/// The overarching event type.
	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
	type TcxId:  Parameter + Member + Default + Bounded + SimpleArithmetic + Copy;
	type TcxType: Parameter + Member + Default + Copy;
	type ActionId: Parameter + Member + Default + Copy;
	type ListingId:  Parameter + Member + Default + Bounded + SimpleArithmetic + Copy;
}

type BalanceOf<T> = <<T as ge::Trait>::Currency as Currency<<T as system::Trait>::AccountId>>::Balance;
type ChallengeId = u32;

#[cfg_attr(feature ="std", derive(Debug, PartialEq, Eq))]
#[derive(Encode, Decode)]
pub struct Tcx<TcxType> {
  pub tcx_type: TcxType,
}


#[derive(Encode, Decode, Default, Clone, PartialEq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct Listing<ListingId, ContentHash, Balance, Moment, AccountId> {
	id: ListingId,
  node_id: ContentHash,
  amount: Balance,
  application_expiry: Moment,
  whitelisted: bool,
  challenge_id: u32,
	owner: AccountId,
}

#[derive(Encode, Decode, Default, Clone, PartialEq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct Challenge<ListingId, Balance, Moment, AccountId> {
  listing_id: ListingId,
  amount: Balance,
  voting_ends: Moment,
  resolved: bool,
  reward_pool: Balance,
  total_tokens: Balance,
	owner: AccountId,
}

#[derive(Encode, Decode, Default, Clone, PartialEq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct Vote<Balance> {
  value: bool,
  amount: Balance,
  claimed: bool,
}

#[derive(Encode, Decode, Default, Clone, PartialEq)]
#[cfg_attr(feature = "std", derive(Debug))]
pub struct Poll<T, U> {
  listing_hash: T,
  votes_for: U,
  votes_against: U,
  passed: bool,
}

// This module's storage items.
decl_storage! {
	trait Store for Module<T: Trait> as Tcx {
    AllTcxsArray get(tcx): map T::TcxId => Option<Tcx<T::TcxType>>;
		AllTcxsCount get(all_tcxs_count): T::TcxId;

    TcxOwner get(owner_of): map T::TcxId => Option<T::GeId>;

    OwnedTcxsArray get(tcx_of_owner_by_index): map (T::GeId, T::TcxId) => T::TcxId;
    OwnedTcxsCount get(owned_tcxs_count): map T::GeId => T::TcxId;

		// actual tcx
    TcxListings get(listing_of_tcr_by_node_id): map (T::TcxId, T::ContentHash) => Listing<T::ListingId, T::ContentHash, BalanceOf<T>, T::Moment, T::AccountId>;
		TcxListingsCount get(listing_count_of_tcr): map T::TcxId => T::ListingId;
    TcxListingsIndexHash get(node_id_of_listing): map (T::TcxId, T::ListingId) => T::ContentHash;

    Challenges get(challenges): map ChallengeId => Challenge<T::ListingId, BalanceOf<T>, T::Moment, T::AccountId>;
    Votes get(votes): map (ChallengeId, T::AccountId) => Vote<BalanceOf<T>>;

	}
}

// The module's dispatchable functions.
decl_module! {
	/// The module declaration.
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		// Initializing events
		// this is needed only if you are using events in your module
		fn deposit_event() = default;

		pub fn propose(origin, tcx_id: T::TcxId, node_id: T::ContentHash, amount: BalanceOf<T>, action_id: T::ActionId) -> Result {
			let who = ensure_signed(origin)?;
			
			// deduction balace for application
			// <token::Module<T>>::lock(sender.clone(), deposit, hashed.clone())?;
			
			// more than min deposit
			let ge_id = Self::owner_of(tcx_id).ok_or("TCX does not exist / TCX owner does not exist")?;
			let governance_entity = <ge::Module<T>>::governance_entity(ge_id).ok_or("GE does not exist")?;
			let min_deposit = governance_entity.min_deposit;
			ensure!(amount >= min_deposit, "deposit should be more than min_deposit");

			let now = <timestamp::Module<T>>::get();
			let apply_stage_len = governance_entity.apply_stage_len;
			let app_exp = now.checked_add(&apply_stage_len).ok_or("Overflow when setting application expiry.")?;

			let listing_id = Self::listing_count_of_tcr(tcx_id);
			let new_listing_count = listing_id.checked_add(&T::ListingId::from(1)).ok_or("Exceed max listing count")?;

			// check action_id

			ensure!(!<TcxListings<T>>::exists((tcx_id,node_id)), "Listing already exists");

			// create a new listing instance
			let new_listing = Listing {
				id: new_listing_count,
				node_id: node_id,
				amount: amount,
				whitelisted: false,
				challenge_id: 0,
				application_expiry: app_exp,
				owner: who.clone(),
			};

			<TcxListings<T>>::insert((tcx_id, node_id), new_listing);
			<TcxListingsCount<T>>::insert(tcx_id, new_listing_count);
			<TcxListingsIndexHash<T>>::insert((tcx_id, new_listing_count), node_id);

			Self::deposit_event(RawEvent::Proposed(who, tcx_id, node_id, amount, action_id));

			Ok(())
		}

		// TODO: node_id or listing_id?
    pub fn challenge(origin, tcx_id: T::TcxId, node_id: T::ContentHash, amount: BalanceOf<T>) -> Result {
			let who = ensure_signed(origin)?;

			let ge_id = Self::owner_of(tcx_id).ok_or("TCX does not exist / TCX owner does not exist")?;
			let governance_entity = <ge::Module<T>>::governance_entity(ge_id).ok_or("GE does not exist")?;

			ensure!(<TcxListings<T>>::exists((tcx_id,node_id)), "Listing not found");
			
			let listing = Self::listing_of_tcr_by_node_id((tcx_id,node_id));
			
			// check if challengable
			ensure!(listing.challenge_id == 0, "Listing is already challenged.");
			// owner - ensure!(listing.owner != sender, "You cannot challenge your own listing.");
			ensure!(amount >= listing.amount, "Amount not enough to challenge");

			let now = <timestamp::Module<T>>::get();
			// check if passed apply stage
			ensure!(listing.application_expiry > now, "Apply stage length has passed.");
			
			let commit_stage_len = governance_entity.commit_stage_len;
			let voting_exp = now.checked_add(&commit_stage_len).ok_or("Overflow when setting voting expiry.")?;

			// let challenge = Challenge {
			// 	listing_hash,
			// 	amount,
			// 	voting_ends: voting_exp,
			// 	resolved: false,
			// 	reward_pool: <T::TokenBalance as As<u64>>::sa(0),
			// 	total_tokens: <T::TokenBalance as As<u64>>::sa(0),
			// 	owner: who.clone(),
			// };

			// let poll = Poll {
			// 	listing_hash,
			// 	votes_for: listing.deposit,
			// 	votes_against: deposit,
			// 	passed: false,
			// };

			// check enough balance, lock it
			// <token::Module<T>>::lock(sender.clone(), deposit, listing_hash)?;

			// // global poll nonce
			// // helps keep the count of challenges and in mapping votes
			// let poll_nonce = <PollNonce<T>>::get();
			// // add a new challenge and the corresponding poll in the respective collections
			// <Challenges<T>>::insert(poll_nonce, challenge);
			// <Polls<T>>::insert(poll_nonce, poll);

			// // update listing with challenge id
			// <Listings<T>>::mutate(listing_hash, |listing| {
			// 	listing.challenge_id = poll_nonce;
			// });

			// // update the poll nonce
			// <PollNonce<T>>::put(poll_nonce + 1);

			Self::deposit_event(RawEvent::Challenged(who, tcx_id, node_id, amount));

			Ok(())
		}

    pub fn vote(origin) -> Result {
			let who = ensure_signed(origin)?;

			// // check if listing is challenged
			// ensure!(<Challenges<T>>::exists(challenge_id), "Challenge does not exist.");
			// let challenge = Self::challenges(challenge_id);
			// ensure!(challenge.resolved == false, "Challenge is already resolved.");

			// // check commit stage length not passed
			// let now = <timestamp::Module<T>>::get();
			// ensure!(challenge.voting_ends > now, "Commit stage length has passed.");

			// // deduct the deposit for vote
			// <token::Module<T>>::lock(sender.clone(), deposit, challenge.listing_hash)?;

			// let mut poll_instance = Self::polls(challenge_id);
			// // based on vote value, increase the count of votes (for or against)
			// match value {
			// 	true => poll_instance.votes_for += deposit,
			// 	false => poll_instance.votes_against += deposit,
			// }

			// // create a new vote instance with the input params
			// let vote_instance = Vote {
			// 	value,
			// 	deposit,
			// 	claimed: false,
			// };

			// // mutate polls collection to update the poll instance
			// <Polls<T>>::mutate(challenge_id, |poll| *poll = poll_instance);

			// // insert new vote into votes collection
			// <Votes<T>>::insert((challenge_id, sender.clone()), vote_instance);

			// // raise the event
			// Self::deposit_event(RawEvent::Voted(sender, challenge_id, deposit));
			Ok(())
		}

    pub fn resolve(origin) -> Result {
			// ensure!(<ListingIndexHash<T>>::exists(listing_id), "Listing not found.");

			// let listing_hash = Self::index_hash(listing_id);
			// let listing = Self::listings(listing_hash);

			// let now = <timestamp::Module<T>>::get();
			// let challenge;
			// let poll;

			// // check if listing is challenged
			// if listing.challenge_id > 0 {
			// 	// challenge
			// 	challenge = Self::challenges(listing.challenge_id);
			// 	poll = Self::polls(listing.challenge_id);

			// 	// check commit stage length has passed
			// 	ensure!(challenge.voting_ends < now, "Commit stage length has not passed.");
			// } else {
			// 	// no challenge
			// 	// check if apply stage length has passed
			// 	ensure!(listing.application_expiry < now, "Apply stage length has not passed.");

			// 	// update listing status
			// 	<Listings<T>>::mutate(listing_hash, |listing|
			// 	{
			// 		listing.whitelisted = true;
			// 	});

			// 	Self::deposit_event(RawEvent::Accepted(listing_hash));
			// 	return Ok(());
			// }

			// let mut whitelisted = false;

			// // mutate polls collection to update the poll instance
			// <Polls<T>>::mutate(listing.challenge_id, |poll| {
			// 	if poll.votes_for >= poll.votes_against {
			// 			poll.passed = true;
			// 			whitelisted = true;
			// 	} else {
			// 			poll.passed = false;
			// 	}
			// });

			// // update listing status
			// <Listings<T>>::mutate(listing_hash, |listing| {
			// 	listing.whitelisted = whitelisted;
			// 	listing.challenge_id = 0;
			// });

			// // update challenge
			// <Challenges<T>>::mutate(listing.challenge_id, |challenge| {
			// 	challenge.resolved = true;
			// 	if whitelisted == true {
			// 		challenge.total_tokens = poll.votes_for;
			// 		challenge.reward_pool = challenge.deposit + poll.votes_against;
			// 	} else {
			// 		challenge.total_tokens = poll.votes_against;
			// 		challenge.reward_pool = listing.deposit + poll.votes_for;
			// 	}
			// });

			// // raise appropriate event as per whitelisting status
			// if whitelisted == true {
			// 	Self::deposit_event(RawEvent::Accepted(listing_hash));
			// } else {
			// 	// if rejected, give challenge deposit back to the challenger
			// 	<token::Module<T>>::unlock(challenge.owner, challenge.deposit, listing_hash)?;
			// 	Self::deposit_event(RawEvent::Rejected(listing_hash));
			// }

			// Self::deposit_event(RawEvent::Resolved(listing_hash, listing.challenge_id));
			Ok(())
		}

    pub fn claim(origin) -> Result {
			let who = ensure_signed(origin)?;

			// // ensure challenge exists and has been resolved
			// ensure!(<Challenges<T>>::exists(challenge_id), "Challenge not found.");
			// let challenge = Self::challenges(challenge_id);
			// ensure!(challenge.resolved == true, "Challenge is not resolved.");

			// // get the poll and vote instances
			// // reward depends on poll passed status and vote value
			// let poll = Self::polls(challenge_id);
			// let vote = Self::votes((challenge_id, sender.clone()));

			// // ensure vote reward is not already claimed
			// ensure!(vote.claimed == false, "Vote reward has already been claimed.");

			// // if winning party, calculate reward and transfer
			// if poll.passed == vote.value {
			// 			let reward_ratio = challenge.reward_pool.checked_div(&challenge.total_tokens).ok_or("overflow in calculating reward")?;
			// 			let reward = reward_ratio.checked_mul(&vote.deposit).ok_or("overflow in calculating reward")?;
			// 			let total = reward.checked_add(&vote.deposit).ok_or("overflow in calculating reward")?;
			// 			<token::Module<T>>::unlock(sender.clone(), total, challenge.listing_hash)?;

			// 			Self::deposit_event(RawEvent::Claimed(sender.clone(), challenge_id));
			// 	}

			// 	// update vote reward claimed status
			// 	<Votes<T>>::mutate((challenge_id, sender), |vote| vote.claimed = true);

			Ok(())
		}
	}
}

decl_event!(
	pub enum Event<T> 
	where 
		AccountId = <T as system::Trait>::AccountId,
		ContentHash = <T as node::Trait>::ContentHash,
		TcxId = <T as Trait>::TcxId,
		ActionId = <T as Trait>::ActionId,
		Balance = <<T as ge::Trait>::Currency as Currency<<T as system::Trait>::AccountId>>::Balance,
	{
		Proposed(AccountId, TcxId, ContentHash, Balance, ActionId),
		Challenged(AccountId, TcxId, ContentHash, Balance),
		Voted(AccountId, u32, Balance),
		Resolved(ContentHash, u32),
		Accepted(ContentHash),
		Rejected(ContentHash),
		Claimed(AccountId, u32),
	}
);

impl<T: Trait> Module<T> {
	pub fn create(ge_id: T::GeId, tcx_type: T::TcxType) -> rstd::result::Result<T::TcxId, &'static str> {
		let one = T::TcxId::from(1 as u32);

		// check global tcx count
		let all_tcxs_count = <AllTcxsCount<T>>::get();
		let new_all_tcxs_count = all_tcxs_count.checked_add(&one).ok_or("Exceed maximum tcx count")?;

		// check owner tcx count
		let owned_tcxs_count = <OwnedTcxsCount<T>>::get(ge_id);
		let new_owned_tcxs_count = owned_tcxs_count.checked_add(&one).ok_or("Exceed maximum tcx count for ge")?;

		let tcx  =  Tcx {
			tcx_type: tcx_type,
		};
		<AllTcxsArray<T>>::insert(new_all_tcxs_count, tcx);
		<AllTcxsCount<T>>::put(new_all_tcxs_count);

		<TcxOwner<T>>::insert(new_all_tcxs_count, ge_id);

		<OwnedTcxsArray<T>>::insert((ge_id, new_owned_tcxs_count), new_all_tcxs_count);
		<OwnedTcxsCount<T>>::insert(ge_id, new_owned_tcxs_count);

		// return new tcx_id
		Ok(new_all_tcxs_count)
	}
}