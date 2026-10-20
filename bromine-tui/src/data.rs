//! What the app knows about the LMS, and how fresh it is.
//!
//! Every fetchable thing has a [`Key`]. Answers are stored in the [`Store`] with the time they were
//! fetched, so views show what they have immediately and the app refetches only what has gone stale
//! (a [`ttl`] per kind of data). The whole store round-trips through JSON: that is the on-disk cache.

use std::collections::{HashMap, HashSet};

use bromine::{
    Announcement, AssignmentDetail, AssignmentRow, CenterItem, Class, ClassFile, ClassInfo, FeedItem, Grades, Person, Topic, TopicItem,
    UserFile,
};
use chrono::{Duration, NaiveDateTime};
use serde::{Deserialize, Serialize};

/// Bumped when the stored shape changes; an older cache is ignored rather than misread.
pub const VERSION: u32 = 1;

const MAX_DETAILS: usize = 300;
const MAX_FEED: usize = 600;
/// After a failed fetch, background polling leaves that key alone for this long.
const RETRY_AFTER: i64 = 60;

/// One thing that can be asked of the LMS.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    Center,
    Classes,
    Feed,
    /// An assignment by `AssignmentIndexId`.
    Detail(i64),
    ClassInfo(i64),
    /// A class's assignments, by lead section id.
    ClassRows(i64),
    Topics(i64),
    TopicItems {
        topic: i64,
        index: i64,
    },
    News(i64),
    ClassFiles(i64),
    Grades(i64),
    People(i64),
    UserFiles,
}

/// What came back for a [`Key`].
#[derive(Debug, Clone)]
pub enum Payload {
    Center(Vec<CenterItem>),
    Classes(Vec<Class>),
    Feed(Vec<FeedItem>),
    Detail(Box<AssignmentDetail>),
    ClassInfo(Box<ClassInfo>),
    ClassRows(Vec<AssignmentRow>),
    Topics(Vec<Topic>),
    TopicItems(Vec<TopicItem>),
    News(Vec<Announcement>),
    ClassFiles(Vec<ClassFile>),
    Grades(Vec<Grades>),
    People(Vec<Person>),
    UserFiles(Vec<UserFile>),
}

/// How long an answer stays fresh.
pub fn ttl(key: &Key) -> Duration {
    Duration::seconds(match key {
        Key::Center | Key::Feed => 60,
        Key::Detail(_) | Key::ClassRows(_) | Key::News(_) => 300,
        Key::Topics(_) | Key::TopicItems { .. } | Key::ClassFiles(_) | Key::Grades(_) => 600,
        Key::Classes | Key::ClassInfo(_) | Key::People(_) | Key::UserFiles => 3600,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry<T> {
    pub value: T,
    pub fetched: NaiveDateTime,
}

/// A view of one stored answer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Load<'a, T> {
    /// Not here yet (never asked, or on its way).
    Loading,
    /// The last attempt failed and there is nothing older to show.
    Failed(&'a str),
    Ready(&'a T),
}

impl<'a, T> Load<'a, T> {
    /// The value, if there is one.
    pub fn ready(self) -> Option<&'a T> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedKind {
    /// From the school's activity feed.
    Post,
    /// A class announcement.
    News,
    /// A topic a teacher published.
    Topic,
}

/// One line of the combined feed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedEntry {
    pub key: String,
    pub kind: FeedKind,
    pub when: Option<NaiveDateTime>,
    pub from: Option<String>,
    pub class: Option<String>,
    /// The `AssignmentIndexId`, when the post is an assignment.
    pub assignment_id: Option<i64>,
    pub title: Option<String>,
    pub text: Option<String>,
}

impl FeedEntry {
    fn post(p: &FeedItem) -> Self {
        let key = match p.id {
            Some(id) => format!("p:{id}"),
            None => format!("p:{:?}:{}", p.when, p.title.as_deref().unwrap_or_default()),
        };
        FeedEntry {
            key,
            kind: FeedKind::Post,
            when: p.when,
            from: p.from.clone(),
            class: p.class.clone(),
            assignment_id: p.assignment_id,
            title: p.title.clone(),
            text: p.text.clone(),
        }
    }

    fn news(n: &Announcement, class: &Class) -> Self {
        FeedEntry {
            key: format!("n:{}:{}:{:?}", class.section_id, n.title, n.published),
            kind: FeedKind::News,
            when: n.published,
            from: n.author.clone().or_else(|| class.teacher.clone()),
            class: Some(class.name.clone()),
            assignment_id: None,
            title: Some(n.title.clone()),
            text: n.text.clone(),
        }
    }

    fn topic(t: &Topic, class: &Class) -> Self {
        FeedEntry {
            key: format!("t:{}", t.topic_id),
            kind: FeedKind::Topic,
            when: t.published,
            from: class.teacher.clone(),
            class: Some(class.name.clone()),
            assignment_id: None,
            title: Some(format!("New topic: {}", t.name)),
            text: None,
        }
    }
}

type Table<T> = HashMap<i64, Entry<T>>;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub version: u32,
    /// Whose data this is: a cache for someone else is not used.
    pub user: Option<String>,
    pub center: Option<Entry<Vec<CenterItem>>>,
    pub classes: Option<Entry<Vec<Class>>>,
    /// Every post the activity feed ever showed us: the server only ever returns the newest ten.
    pub posts: Option<Entry<Vec<FeedEntry>>>,
    pub details: Table<AssignmentDetail>,
    pub class_info: Table<ClassInfo>,
    pub class_rows: Table<Vec<AssignmentRow>>,
    pub topics: Table<Vec<Topic>>,
    pub topic_items: Table<Vec<TopicItem>>,
    pub news: Table<Vec<Announcement>>,
    pub class_files: Table<Vec<ClassFile>>,
    pub grades: Table<Vec<Grades>>,
    pub people: Table<Vec<Person>>,
    pub user_files: Option<Entry<Vec<UserFile>>>,

    #[serde(skip)]
    pub loading: HashSet<Key>,
    #[serde(skip)]
    pub errors: HashMap<Key, String>,
    #[serde(skip)]
    failed_at: HashMap<Key, NaiveDateTime>,
}

impl Store {
    pub fn new(user: Option<String>) -> Self {
        Store { version: VERSION, user, ..Store::default() }
    }

    /// A stored cache is only good for the version and user that wrote it.
    pub fn usable_for(&self, user: Option<&str>) -> bool {
        self.version == VERSION && (user.is_none() || self.user.as_deref() == user)
    }

    pub fn items(&self) -> &[CenterItem] {
        self.center.as_ref().map_or(&[], |e| &e.value)
    }

    pub fn items_mut(&mut self) -> &mut Vec<CenterItem> {
        &mut self.center.get_or_insert_with(|| Entry { value: vec![], fetched: NaiveDateTime::MIN }).value
    }

    pub fn class_list(&self) -> &[Class] {
        self.classes.as_ref().map_or(&[], |e| &e.value)
    }

    /// When this answer was fetched, if we have one.
    pub fn fetched(&self, key: &Key) -> Option<NaiveDateTime> {
        match key {
            Key::Center => self.center.as_ref().map(|e| e.fetched),
            Key::Classes => self.classes.as_ref().map(|e| e.fetched),
            Key::Feed => self.posts.as_ref().map(|e| e.fetched),
            Key::Detail(id) => self.details.get(id).map(|e| e.fetched),
            Key::ClassInfo(s) => self.class_info.get(s).map(|e| e.fetched),
            Key::ClassRows(s) => self.class_rows.get(s).map(|e| e.fetched),
            Key::Topics(s) => self.topics.get(s).map(|e| e.fetched),
            Key::TopicItems { topic, .. } => self.topic_items.get(topic).map(|e| e.fetched),
            Key::News(s) => self.news.get(s).map(|e| e.fetched),
            Key::ClassFiles(s) => self.class_files.get(s).map(|e| e.fetched),
            Key::Grades(s) => self.grades.get(s).map(|e| e.fetched),
            Key::People(s) => self.people.get(s).map(|e| e.fetched),
            Key::UserFiles => self.user_files.as_ref().map(|e| e.fetched),
        }
    }

    /// Should `key` be (re)fetched now? Not while it is in flight; not soon after a failure unless `force`.
    pub fn needs(&self, key: &Key, now: NaiveDateTime, force: bool) -> bool {
        if self.loading.contains(key) {
            return false;
        }
        if force {
            return true;
        }
        if self.failed_at.get(key).is_some_and(|t| now - *t < Duration::seconds(RETRY_AFTER)) {
            return false;
        }
        self.fetched(key).is_none_or(|t| now - t >= ttl(key))
    }

    pub fn fail(&mut self, key: &Key, error: String, now: NaiveDateTime) {
        self.loading.remove(key);
        self.errors.insert(key.clone(), error);
        self.failed_at.insert(key.clone(), now);
    }

    /// Store an answer. An answer that does not match its key is dropped.
    pub fn apply(&mut self, key: &Key, payload: Payload, now: NaiveDateTime) {
        let stored = match (key, payload) {
            (Key::Center, Payload::Center(v)) => {
                self.center = Some(Entry { value: v, fetched: now });
                true
            }
            (Key::Classes, Payload::Classes(v)) => {
                self.classes = Some(Entry { value: v, fetched: now });
                true
            }
            (Key::Feed, Payload::Feed(v)) => {
                self.merge_posts(&v, now);
                true
            }
            (Key::Detail(id), Payload::Detail(v)) => {
                self.details.insert(*id, Entry { value: *v, fetched: now });
                true
            }
            (Key::ClassInfo(s), Payload::ClassInfo(v)) => {
                self.class_info.insert(*s, Entry { value: *v, fetched: now });
                true
            }
            (Key::ClassRows(s), Payload::ClassRows(v)) => {
                self.class_rows.insert(*s, Entry { value: v, fetched: now });
                true
            }
            (Key::Topics(s), Payload::Topics(v)) => {
                self.topics.insert(*s, Entry { value: v, fetched: now });
                true
            }
            (Key::TopicItems { topic, .. }, Payload::TopicItems(v)) => {
                self.topic_items.insert(*topic, Entry { value: v, fetched: now });
                true
            }
            (Key::News(s), Payload::News(v)) => {
                self.news.insert(*s, Entry { value: v, fetched: now });
                true
            }
            (Key::ClassFiles(s), Payload::ClassFiles(v)) => {
                self.class_files.insert(*s, Entry { value: v, fetched: now });
                true
            }
            (Key::Grades(s), Payload::Grades(v)) => {
                self.grades.insert(*s, Entry { value: v, fetched: now });
                true
            }
            (Key::People(s), Payload::People(v)) => {
                self.people.insert(*s, Entry { value: v, fetched: now });
                true
            }
            (Key::UserFiles, Payload::UserFiles(v)) => {
                self.user_files = Some(Entry { value: v, fetched: now });
                true
            }
            _ => false,
        };
        debug_assert!(stored, "payload does not match {key:?}");
        self.loading.remove(key);
        if stored {
            self.errors.remove(key);
            self.failed_at.remove(key);
        }
    }

    /// Fold the newest posts into the archive: new ones are added, ones we already had are refreshed.
    fn merge_posts(&mut self, fresh: &[FeedItem], now: NaiveDateTime) {
        let mut all = self.posts.take().map(|e| e.value).unwrap_or_default();
        for entry in fresh.iter().map(FeedEntry::post) {
            match all.iter_mut().find(|e| e.key == entry.key) {
                Some(old) => *old = entry,
                None => all.push(entry),
            }
        }
        all.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| a.key.cmp(&b.key)));
        all.truncate(MAX_FEED);
        self.posts = Some(Entry { value: all, fetched: now });
    }

    /// The feed as the person sees it: the archived posts, every class's announcements and every
    /// topic, newest first. An announcement the activity feed already carried is not shown twice.
    pub fn combined_feed(&self) -> Vec<FeedEntry> {
        let mut out: Vec<FeedEntry> = self.posts.as_ref().map(|e| e.value.clone()).unwrap_or_default();
        let norm = |s: &str| s.trim().to_lowercase();
        let seen: HashSet<(String, String)> =
            out.iter().map(|p| (norm(p.class.as_deref().unwrap_or_default()), norm(p.title.as_deref().unwrap_or_default()))).collect();
        for class in self.class_list() {
            for n in self.news.get(&class.section_id).map(|e| e.value.as_slice()).unwrap_or_default() {
                if !seen.contains(&(norm(&class.name), norm(&n.title))) {
                    out.push(FeedEntry::news(n, class));
                }
            }
            for t in self.topics.get(&class.section_id).map(|e| e.value.as_slice()).unwrap_or_default() {
                out.push(FeedEntry::topic(t, class));
            }
        }
        out.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| a.key.cmp(&b.key)));
        out.truncate(MAX_FEED);
        out
    }

    fn view<'a, T>(&'a self, entry: Option<&'a Entry<T>>, key: &Key) -> Load<'a, T> {
        match (entry, self.errors.get(key)) {
            (Some(e), _) => Load::Ready(&e.value),
            (None, Some(err)) if !self.loading.contains(key) => Load::Failed(err),
            (None, _) => Load::Loading,
        }
    }

    pub fn detail(&self, id: i64) -> Load<'_, AssignmentDetail> {
        self.view(self.details.get(&id), &Key::Detail(id))
    }
    pub fn class_rows(&self, lead: i64) -> Load<'_, Vec<AssignmentRow>> {
        self.view(self.class_rows.get(&lead), &Key::ClassRows(lead))
    }
    pub fn topics(&self, section: i64) -> Load<'_, Vec<Topic>> {
        self.view(self.topics.get(&section), &Key::Topics(section))
    }
    pub fn topic_items(&self, topic: &Topic) -> Load<'_, Vec<TopicItem>> {
        self.view(self.topic_items.get(&topic.topic_id), &Key::TopicItems { topic: topic.topic_id, index: topic.index_id })
    }
    pub fn news(&self, section: i64) -> Load<'_, Vec<Announcement>> {
        self.view(self.news.get(&section), &Key::News(section))
    }
    pub fn class_files(&self, section: i64) -> Load<'_, Vec<ClassFile>> {
        self.view(self.class_files.get(&section), &Key::ClassFiles(section))
    }
    pub fn grades(&self, section: i64) -> Load<'_, Vec<Grades>> {
        self.view(self.grades.get(&section), &Key::Grades(section))
    }
    pub fn people(&self, section: i64) -> Load<'_, Vec<Person>> {
        self.view(self.people.get(&section), &Key::People(section))
    }
    pub fn user_files(&self) -> Load<'_, Vec<UserFile>> {
        self.view(self.user_files.as_ref(), &Key::UserFiles)
    }

    /// Keep the on-disk cache bounded: only the most recently fetched assignment details.
    pub fn prune(&mut self) {
        if self.details.len() > MAX_DETAILS {
            let mut by_age: Vec<(NaiveDateTime, i64)> = self.details.iter().map(|(id, e)| (e.fetched, *id)).collect();
            by_age.sort();
            for (_, id) in by_age.into_iter().take(self.details.len() - MAX_DETAILS) {
                self.details.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 20).unwrap().and_hms_opt(h, m, s).unwrap()
    }

    fn post(id: i64, title: &str, h: u32) -> FeedItem {
        FeedItem {
            id: Some(id),
            ticks: Some(id),
            when: Some(at(h, 0, 0)),
            from: Some("T".into()),
            class: Some("BIOLOGY - 20".into()),
            assignment_id: None,
            title: Some(title.into()),
            text: None,
        }
    }

    fn class(name: &str, section: i64) -> Class {
        Class { name: name.into(), section_id: section, lead_section_id: section, teacher: Some("Ms. A".into()), subject: None }
    }

    #[test]
    fn data_is_stale_after_its_ttl_and_never_while_loading() {
        let mut s = Store::new(None);
        let t0 = at(10, 0, 0);
        assert!(s.needs(&Key::Center, t0, false), "never fetched");
        s.loading.insert(Key::Center);
        assert!(!s.needs(&Key::Center, t0, true), "already on its way, even when forced");
        s.apply(&Key::Center, Payload::Center(vec![]), t0);
        assert!(!s.loading.contains(&Key::Center));
        assert!(!s.needs(&Key::Center, at(10, 0, 59), false));
        assert!(s.needs(&Key::Center, at(10, 1, 0), false), "centers go stale after a minute");
        assert!(s.needs(&Key::Center, at(10, 0, 1), true), "force ignores freshness");
        s.apply(&Key::Classes, Payload::Classes(vec![]), t0);
        assert!(!s.needs(&Key::Classes, at(10, 59, 0), false), "classes change rarely: an hour");
    }

    #[test]
    fn a_failure_backs_polling_off_but_not_a_forced_refresh_and_a_success_clears_it() {
        let mut s = Store::new(None);
        let t0 = at(10, 0, 0);
        s.loading.insert(Key::Feed);
        s.fail(&Key::Feed, "boom".into(), t0);
        assert!(matches!(s.view(None::<&Entry<()>>, &Key::Feed), Load::Failed("boom")));
        assert!(!s.needs(&Key::Feed, at(10, 0, 30), false), "polling waits a minute");
        assert!(s.needs(&Key::Feed, at(10, 0, 30), true));
        assert!(s.needs(&Key::Feed, at(10, 1, 1), false));
        s.apply(&Key::Feed, Payload::Feed(vec![]), at(10, 2, 0));
        assert!(s.errors.is_empty() && matches!(s.view(s.posts.as_ref(), &Key::Feed), Load::Ready(_)));
    }

    #[test]
    fn old_answers_stay_visible_while_a_refresh_is_in_flight_or_failed() {
        let mut s = Store::new(None);
        s.apply(&Key::Detail(5), Payload::Detail(Box::new(crate::demo::detail(&crate::demo::items(at(9, 0, 0))[0]))), at(9, 0, 0));
        s.loading.insert(Key::Detail(5));
        assert!(matches!(s.detail(5), Load::Ready(_)));
        s.fail(&Key::Detail(5), "offline".into(), at(9, 10, 0));
        assert!(matches!(s.detail(5), Load::Ready(_)), "stale beats an error message");
        assert!(matches!(s.detail(6), Load::Loading));
    }

    #[test]
    fn the_feed_archive_grows_past_what_the_server_returns() {
        let mut s = Store::new(None);
        s.apply(&Key::Feed, Payload::Feed(vec![post(1, "a", 8), post(2, "b", 9)]), at(10, 0, 0));
        s.apply(&Key::Feed, Payload::Feed(vec![post(2, "b (edited)", 9), post(3, "c", 10)]), at(10, 5, 0));
        let titles: Vec<_> = s.posts.as_ref().unwrap().value.iter().map(|p| p.title.clone().unwrap()).collect();
        assert_eq!(titles, ["c", "b (edited)", "a"], "newest first, no duplicates, the oldest is kept");
    }

    #[test]
    fn the_combined_feed_adds_announcements_and_topics_without_doubling_up() {
        let mut s = Store::new(None);
        let t = at(12, 0, 0);
        s.apply(&Key::Classes, Payload::Classes(vec![class("BIOLOGY - 20", 1), class("ENGLISH - 20", 2)]), t);
        s.apply(&Key::Feed, Payload::Feed(vec![post(1, "Room change", 8)]), t);
        let ann = |title: &str, h: u32| Announcement { title: title.into(), author: None, published: Some(at(h, 0, 0)), text: None };
        s.apply(&Key::News(1), Payload::News(vec![ann("room change", 8), ann("Goggles", 11)]), t);
        s.apply(&Key::News(2), Payload::News(vec![ann("Reading", 7)]), t);
        s.apply(
            &Key::Topics(2),
            Payload::Topics(vec![Topic { topic_id: 9, index_id: 9, name: "Unit 1".into(), published: Some(at(6, 0, 0)) }]),
            t,
        );
        let feed = s.combined_feed();
        let line: Vec<_> = feed.iter().map(|f| (f.kind, f.title.clone().unwrap())).collect();
        assert_eq!(
            line,
            [
                (FeedKind::News, "Goggles".into()),
                (FeedKind::Post, "Room change".into()),
                (FeedKind::News, "Reading".into()),
                (FeedKind::Topic, "New topic: Unit 1".into()),
            ],
            "the announcement the activity feed already had is dropped"
        );
        assert_eq!(feed[0].from.as_deref(), Some("Ms. A"), "an announcement with no author is credited to the teacher");
    }

    #[test]
    fn the_store_round_trips_and_transient_state_is_not_saved() {
        let mut s = Store::new(Some("student".into()));
        s.apply(&Key::Center, Payload::Center(crate::demo::items(at(9, 0, 0))), at(9, 0, 0));
        s.loading.insert(Key::Classes);
        s.errors.insert(Key::Feed, "x".into());
        let back: Store = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.items(), s.items());
        assert!(back.loading.is_empty() && back.errors.is_empty());
        assert!(back.usable_for(Some("student")) && !back.usable_for(Some("other")) && back.usable_for(None));
        let mut old = Store::new(None);
        old.version = 0;
        assert!(!old.usable_for(None), "a cache from another version is ignored");
    }

    #[test]
    fn only_the_newest_details_are_kept_on_disk() {
        let mut s = Store::new(None);
        let d = crate::demo::detail(&crate::demo::items(at(9, 0, 0))[0]);
        for i in 0..(MAX_DETAILS as i64 + 20) {
            s.details.insert(i, Entry { value: d.clone(), fetched: at(9, 0, 0) + Duration::seconds(i) });
        }
        s.prune();
        assert_eq!(s.details.len(), MAX_DETAILS);
        assert!(s.details.contains_key(&(MAX_DETAILS as i64 + 19)) && !s.details.contains_key(&0));
    }
}
