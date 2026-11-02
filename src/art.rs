//! Album art decoded off the drawing thread.
//!
//! A scan stores a cover's encoded bytes and keeps only the key it was stored
//! under, so listing a library of thousands of tracks decodes no picture. One
//! thread decodes them here instead, taking two orders of work: a picture the
//! window is about to draw, which is decoded next, and a bulk list of the rest,
//! which is worked through behind the frames. The thread looks for drawn work
//! before it touches the bulk, so a key the window asks for never waits behind
//! a whole library.
//!
//! `Art::prefetch` queues the whole library when one lands, so a cover is
//! decoded whether or not its row is ever on screen, and only a picture the
//! window asked for makes it repaint. A key is decoded once and its answer is
//! remembered, so a picture is decoded to be drawn or behind the frames but
//! never twice.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use crate::cover::Cover;

/// How many decoded pictures are kept.
///
/// A cover is a quarter of a megabyte decoded, so a library of thousands of
/// albums would hold hundreds of megabytes if every one stayed. Past this the
/// picture decoded longest ago goes, and a key drawn again is decoded again.
const KEPT: usize = 512;

/// The decoded pictures, and the thread that decodes the rest.
#[derive(Default)]
pub struct Art {
    /// Pictures by key.
    ///
    /// `None` is a key with no picture behind it, which is remembered so the
    /// key is not asked for again until a library is taken in again.
    decoded: HashMap<String, Option<Arc<Cover>>>,
    /// The keys of those pictures, oldest first, so one can make room.
    order: VecDeque<String>,
    /// Keys handed to the thread, so one is asked for once.
    asked: HashSet<String>,
    /// The keys among those the window is waiting to draw.
    drawn: HashSet<String>,
    /// The thread, started when the first key is asked for.
    worker: Option<Worker>,
}

/// What the window asks the thread for.
enum Request {
    /// A picture the window is about to draw, which is decoded next.
    Drawn(String),
    /// Pictures to decode behind the frames, in the order given.
    Bulk(Vec<String>),
    /// Keys the window forgot, so the thread decodes them again when asked.
    Forget(Vec<String>),
}

/// The queues to the thread that decodes pictures.
struct Worker {
    /// What to decode.
    wanted: Sender<Request>,
    /// Pictures that came back.
    results: Receiver<(String, Option<Arc<Cover>>)>,
}

impl Art {
    /// The picture for `key`, when it has been decoded.
    ///
    /// The outer `None` is a picture not decoded yet; the inner one is a key
    /// with no picture behind it, which the window draws as a placeholder.
    pub fn decoded(&self, key: &str) -> Option<Option<Arc<Cover>>> {
        self.decoded.get(key).cloned()
    }

    /// Asks the thread for `key` ahead of the bulk list.
    ///
    /// A key asked for this way is one the window is about to draw, so its
    /// arrival makes the window repaint, and it is decoded before anything
    /// `prefetch` queued. A key already handed over by `prefetch` is asked for
    /// again rather than left behind the whole library, and one the thread has
    /// decoded already is turned away on a hash lookup, so asking again on
    /// every frame until the picture lands costs a message, not a decode.
    pub fn request(&mut self, key: &str) {
        if self.decoded.contains_key(key) {
            return;
        }
        self.drawn.insert(key.to_string());
        self.asked.insert(key.to_string());
        let worker = self.worker.get_or_insert_with(Worker::start);
        let _ = worker.wanted.send(Request::Drawn(key.to_string()));
    }

    /// Hands the thread every key the window is not showing, in one go.
    ///
    /// This is what fills the pictures behind the visible rows: the thread
    /// works through them after whatever the window asks for, so a cover is
    /// decoded whether or not its row is ever painted, and a scroll finds the
    /// ones above it already done.
    pub fn prefetch(&mut self, keys: impl IntoIterator<Item = String>) {
        let mut bulk = Vec::new();
        for key in keys {
            if !self.decoded.contains_key(&key) && self.asked.insert(key.clone()) {
                bulk.push(key);
            }
        }
        if bulk.is_empty() {
            return;
        }
        let worker = self.worker.get_or_insert_with(Worker::start);
        let _ = worker.wanted.send(Request::Bulk(bulk));
    }

    /// Takes in every picture that has come back.
    ///
    /// Says whether one the window asked for arrived, which is the only thing
    /// worth a repaint: the pictures `prefetch` queued behind them are found
    /// by the frame that draws them.
    pub fn poll(&mut self) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        let mut drawn = false;
        while let Ok((key, cover)) = worker.results.try_recv() {
            drawn |= self.drawn.remove(&key);
            if let Entry::Vacant(slot) = self.decoded.entry(key.clone()) {
                slot.insert(cover);
                self.order.push_back(key);
            }
        }
        self.make_room();
        drawn
    }

    /// Whether a picture the window is waiting to draw is still decoding.
    ///
    /// Work behind the visible rows is not counted: it costs no frames, and
    /// arrives by the time the frame that draws it runs.
    pub fn pending(&self) -> bool {
        !self.drawn.is_empty()
    }

    /// Drops the pictures and keys of covers `keep` does not name.
    ///
    /// A library that is swapped in usually names most of the same pictures,
    /// so keeping those saves decoding them again and re-uploading them as
    /// textures, while the ones the old library had go.
    pub fn retain(&mut self, keep: &HashSet<String>) {
        let dropped: Vec<String> = self
            .decoded
            .keys()
            .filter(|key| !keep.contains(*key))
            .cloned()
            .collect();
        self.forget(&dropped);
    }

    /// Forgets the keys that answered with no picture.
    ///
    /// A new library is the moment to ask again: art that was missing from the
    /// cache may have been stored since, and an answer held from before it
    /// would leave the window drawing a placeholder for it.
    pub fn forget_missing(&mut self) {
        let missing: Vec<String> = self
            .decoded
            .iter()
            .filter(|(_, cover)| cover.is_none())
            .map(|(key, _)| key.clone())
            .collect();
        self.forget(&missing);
    }

    /// Drops `keys` here and in the thread, so asking for one decodes it again.
    ///
    /// The thread remembers what it has decoded too. Leaving a key there while
    /// dropping it here would swallow the next ask for it, which is what a
    /// picture evicted for room and asked for again runs into. The window
    /// calls this on its own for a picture it uploaded at a size of its own:
    /// that one is a few megabytes, and the texture it became is all the
    /// window needs.
    pub fn forget(&mut self, keys: &[String]) {
        for key in keys {
            self.decoded.remove(key);
            self.asked.remove(key);
            self.drawn.remove(key);
        }
        self.order.retain(|key| self.decoded.contains_key(key));
        if keys.is_empty() {
            return;
        }
        if let Some(worker) = &self.worker {
            let _ = worker.wanted.send(Request::Forget(keys.to_vec()));
        }
    }

    /// Drops the pictures decoded longest ago once too many are kept.
    fn make_room(&mut self) {
        let excess = self.decoded.len().saturating_sub(KEPT);
        if excess == 0 {
            return;
        }
        let dropped: Vec<String> = self.order.drain(..excess.min(self.order.len())).collect();
        // The keys may be asked for again, and this time they are worth
        // decoding: what dropped them was room, not a missing picture.
        self.forget(&dropped);
    }
}

impl Worker {
    /// Starts the thread, which ends when the app drops its end of the queue.
    fn start() -> Self {
        let (wanted, queue) = mpsc::channel::<Request>();
        let (sender, results) = mpsc::channel();
        thread::spawn(move || {
            let mut backlog: VecDeque<String> = VecDeque::new();
            let mut done: HashSet<String> = HashSet::new();
            loop {
                match queue.try_recv() {
                    Ok(request) => handle(request, &mut backlog, &mut done, &sender),
                    Err(TryRecvError::Empty) => match backlog.pop_front() {
                        Some(key) => decode(key, &mut done, &sender),
                        None => match queue.recv() {
                            Ok(request) => handle(request, &mut backlog, &mut done, &sender),
                            Err(_) => return,
                        },
                    },
                    Err(TryRecvError::Disconnected) => return,
                }
            }
        });
        Self { wanted, results }
    }
}

/// Carries out one request from the window.
fn handle(
    request: Request,
    backlog: &mut VecDeque<String>,
    done: &mut HashSet<String>,
    sender: &Sender<(String, Option<Arc<Cover>>)>,
) {
    match request {
        Request::Drawn(key) => {
            // The window asks again on every frame until the picture arrives,
            // so an answer already given is turned away before the backlog is
            // walked: a pass over a whole library per frame is what scrolling
            // during a prefetch would otherwise cost.
            if done.contains(&key) {
                return;
            }
            // The bulk list may still hold it from a prefetch; a drawn key is
            // decoded now, so it is taken out rather than decoded twice.
            backlog.retain(|queued| *queued != key);
            decode(key, done, sender);
        }
        Request::Bulk(keys) => backlog.extend(keys),
        Request::Forget(keys) => {
            for key in keys {
                done.remove(&key);
                backlog.retain(|queued| *queued != key);
            }
        }
    }
}

/// Decodes `key` and sends the answer on, unless it was decoded already.
///
/// The window asks for a picture on every frame until it arrives, so this is
/// what keeps that from decoding it once per frame.
fn decode(key: String, done: &mut HashSet<String>, sender: &Sender<(String, Option<Arc<Cover>>)>) {
    if !done.insert(key.clone()) {
        return;
    }
    let cover = decode_now(&key).map(Arc::new);
    let _ = sender.send((key, cover));
}

/// Decodes the picture stored under `key`, on the calling thread.
///
/// The media widget names a track's art by a file, so it needs the pixels at
/// once rather than a frame later, and a track is played once at a time.
pub fn decode_now(key: &str) -> Option<Cover> {
    let (key, size) = split(key);
    let bytes = crate::cache::read_cover(key)?;
    match size {
        Some(size) => crate::cover::decode_at(&bytes, size),
        None => crate::cover::decode(&bytes),
    }
}

/// The key a picture is stored under, and the size it was asked for.
///
/// A cover the window draws large is stored under its key with the size after
/// it, `0123456789abcdef-2a1@640`, so the same picture can be decoded at
/// thumbnail size for the album lists and at the size of the big square for
/// the track that plays without the two answering for each other.
const SIZE_MARK: char = '@';

/// The stored key and the size asked for, which a plain key does not name.
fn split(key: &str) -> (&str, Option<u32>) {
    match key.split_once(SIZE_MARK) {
        Some((key, size)) => (key, size.parse().ok()),
        None => (key, None),
    }
}

/// The key that asks for the picture `key` at `size` pixels.
pub fn sized_key(key: &str, size: u32) -> String {
    format!("{key}{SIZE_MARK}{size}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Polls `art` until `key` has an answer, or the test gives up.
    fn settle(art: &mut Art, key: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            art.poll();
            if art.decoded(key).is_some() || Instant::now() > deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A stored picture that differs from every other, and its key.
    fn stored(n: usize) -> String {
        let cover = Cover {
            width: 4,
            height: 4,
            rgba: [n as u8, (n >> 8) as u8, (n >> 16) as u8, 255].repeat(16),
        };
        let bytes = crate::cover::to_png(&cover).expect("the picture encodes");
        crate::cache::store_cover(&bytes).expect("the picture is stored")
    }

    #[test]
    fn a_key_that_names_a_size_is_decoded_at_it() {
        // The square above the controls draws a cover wider than a list row,
        // so it asks for the same picture at a size of its own instead of
        // stretching the thumbnail the lists use.
        let cover = Cover {
            width: 800,
            height: 800,
            rgba: [30, 60, 90, 255].repeat(800 * 800),
        };
        let bytes = crate::cover::to_png(&cover).expect("the picture encodes");
        let key = crate::cache::store_cover(&bytes).expect("the picture is stored");

        let thumbnail = decode_now(&key).expect("the thumbnail decodes");
        assert_eq!(
            (thumbnail.width, thumbnail.height),
            (256, 256),
            "a plain key is a thumbnail"
        );
        let large = decode_now(&sized_key(&key, 640)).expect("the big picture decodes");
        assert_eq!(
            (large.width, large.height),
            (640, 640),
            "a key naming a size decodes at that size"
        );
    }

    #[test]
    fn nothing_is_decoded_before_it_is_asked_for() {
        let art = Art::default();
        assert!(art.decoded("0000000000000000-0").is_none());
        assert!(!art.pending());
    }

    #[test]
    fn a_key_with_no_picture_is_answered_once_and_never_asked_for_again() {
        let mut art = Art::default();
        let key = "0000000000000000-0";
        art.request(key);
        settle(&mut art, key);

        assert!(
            art.decoded(key).is_some_and(|cover| cover.is_none()),
            "a key whose bytes are gone answers with no picture"
        );
        art.request(key);
        assert!(!art.pending(), "the answer is kept, so it is not retried");
    }

    #[test]
    fn a_prefetched_key_is_answered_without_being_drawn() {
        let mut art = Art::default();
        let key = stored(10);
        art.prefetch([key.clone()]);
        assert!(
            !art.pending(),
            "work the window is not waiting for costs it no frames"
        );
        settle(&mut art, &key);

        assert!(
            art.decoded(&key).is_some_and(|cover| cover.is_some()),
            "decoded"
        );
        assert!(!art.pending(), "and the thread is idle again");
    }

    #[test]
    fn a_drawn_picture_does_not_wait_for_the_library_behind_it() {
        // The thread may already be decoding the first key when the window
        // asks for one, so what this pins is that the ask jumps the queue
        // rather than sitting at its end: a few may land first, never dozens.
        let keys: Vec<String> = (0..40).map(|shade| stored(shade * 5)).collect();
        let mut art = Art::default();
        art.prefetch(keys.clone());
        let last = keys.last().expect("the list is not empty").clone();
        art.request(&last);

        let deadline = Instant::now() + Duration::from_secs(5);
        while art.decoded(&last).is_none() && Instant::now() < deadline {
            art.poll();
            std::thread::sleep(Duration::from_millis(1));
        }

        assert!(
            art.decoded(&last).is_some(),
            "the picture the window asked for arrived"
        );
        let place = art
            .order
            .iter()
            .position(|key| *key == last)
            .expect("the picture arrived");
        assert!(
            place < 5,
            "the ask jumped the queue: it arrived at {place} of {}",
            art.order.len()
        );
    }

    #[test]
    fn a_key_drawn_after_it_was_prefetched_is_decoded_once() {
        let key = stored(20);
        let mut art = Art::default();
        art.prefetch([key.clone()]);
        art.request(&key);
        settle(&mut art, &key);
        art.poll();

        assert!(art.decoded(&key).is_some_and(|cover| cover.is_some()));
        assert!(!art.pending(), "nothing is left waiting");
    }

    #[test]
    fn forgetting_the_missing_asks_for_those_keys_again() {
        let art_key = stored(30);
        let mut art = Art::default();
        art.request("0000000000000000-0");
        art.request(&art_key);
        settle(&mut art, "0000000000000000-0");
        settle(&mut art, &art_key);

        art.forget_missing();

        assert!(
            art.decoded("0000000000000000-0").is_none(),
            "a key with no picture is asked for again"
        );
        assert!(
            art.decoded(&art_key).is_some_and(|cover| cover.is_some()),
            "a decoded picture is kept"
        );
    }

    #[test]
    fn a_key_asked_for_twice_is_queued_once() {
        let key = stored(40);
        let mut art = Art::default();
        art.prefetch([key.clone(), key.clone()]);
        art.request(&key);
        settle(&mut art, &key);

        assert!(!art.pending());
        assert_eq!(art.asked.len(), 1, "one key, one entry");
    }

    #[test]
    fn the_pictures_decoded_longest_ago_go_first() {
        let mut art = Art::default();
        for index in 0..=KEPT {
            let key = format!("{index:016x}-0");
            art.order.push_back(key.clone());
            art.decoded.insert(key.clone(), None);
            art.asked.insert(key);
        }

        art.make_room();

        assert_eq!(art.decoded.len(), KEPT, "only the excess goes");
        assert!(
            !art.decoded.contains_key("0000000000000000-0"),
            "the picture decoded first is the one dropped"
        );
        assert!(
            art.decoded.contains_key(&format!("{KEPT:016x}-0")),
            "the newest one stays"
        );
        assert!(
            !art.asked.contains("0000000000000000-0"),
            "a dropped key may be decoded again"
        );
    }

    #[test]
    fn a_picture_evicted_for_room_is_decoded_again_when_asked() {
        // The thread remembers what it decoded, so a key dropped here has to
        // be dropped there too: otherwise the ask is swallowed, the window
        // waits for a picture nobody sends, and it repaints for good.
        let keys: Vec<String> = (0..=KEPT).map(stored).collect();
        let mut art = Art::default();
        art.prefetch(keys.clone());
        let deadline = Instant::now() + Duration::from_secs(10);
        let first = keys[0].clone();
        // `make_room` caps `decoded` at `KEPT`, so waiting for a length past
        // the cap would sleep out the whole deadline: what says the eviction
        // happened is the cap being reached and the first key having gone.
        while (art.order.len() < KEPT || art.decoded(&first).is_some()) && Instant::now() < deadline
        {
            art.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            art.order.len() >= KEPT,
            "the cap was reached before the deadline, so eviction happened"
        );
        assert!(
            art.decoded(&first).is_none(),
            "the first picture was evicted for room"
        );

        art.request(&first);
        settle(&mut art, &first);

        assert!(
            art.decoded(&first).is_some_and(|cover| cover.is_some()),
            "the evicted picture is decoded again"
        );
        assert!(!art.pending(), "and the window stops waiting for it");
    }
}
