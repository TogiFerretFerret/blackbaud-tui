//! A fake, in-memory LMS: `bromine-tui --demo` runs the whole UI against it (no network, no login),
//! and the tests use its data.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use bromine::{
    Announcement, AssignmentDetail, AssignmentRow, Bucket, CenterItem, Class, ClassFile, ClassInfo, FeedItem, FileRef, Flags, GradeRow,
    Grades, ItemKind, Person, Status, Submission, TaskPatch, Term, Topic, TopicItem, UserFile,
};
use chrono::{Duration as Dur, NaiveDateTime};

use crate::data::{Key, Payload};
use crate::worker::{Cmd, Event};

const CLASSES: [(&str, i64, &str); 5] = [
    ("BIOLOGY - 20", 101, "Ms. Alvarez"),
    ("ENGLISH III - 20", 102, "Mr. Brooks"),
    ("PRECALCULUS - 10", 103, "Dr. Chen"),
    ("SPANISH III - 20", 104, "Sra. Duarte"),
    ("US HISTORY - 20", 105, "Mx. Evans"),
];

pub fn classes() -> Vec<Class> {
    CLASSES
        .iter()
        .map(|(name, id, teacher)| Class {
            name: (*name).into(),
            section_id: *id,
            lead_section_id: *id,
            teacher: Some((*teacher).into()),
            subject: None,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn item(
    bucket: Bucket,
    id: i64,
    class: usize,
    name: &str,
    kind: &str,
    due: NaiveDateTime,
    status: Status,
    points: f64,
    major: bool,
) -> CenterItem {
    CenterItem {
        bucket,
        kind: ItemKind::Assignment,
        id,
        assignment_id: id + 1000,
        section_id: CLASSES[class].1,
        class: CLASSES[class].0.into(),
        name: name.into(),
        assignment_type: Some(kind.into()),
        due: Some(due),
        assigned: Some(due - Dur::days(5)),
        status,
        points: Some(points),
        major,
        flags: Flags::default(),
        has_download: false,
    }
}

pub fn task(id: i64, name: &str, due: NaiveDateTime, bucket: Bucket) -> CenterItem {
    CenterItem {
        bucket,
        kind: ItemKind::Task,
        id,
        assignment_id: 0,
        section_id: 0,
        class: "General Task".into(),
        name: name.into(),
        assignment_type: Some("My tasks".into()),
        due: Some(due),
        assigned: None,
        status: Status::Todo,
        points: None,
        major: false,
        flags: Flags::default(),
        has_download: false,
    }
}

/// An assignment center shaped like a real one, with dates relative to `now`.
pub fn items(now: NaiveDateTime) -> Vec<CenterItem> {
    let at = |days: i64, h: u32| (now.date() + Dur::days(days)).and_hms_opt(h, 0, 0).unwrap();
    let mut v = vec![
        item(Bucket::DueToday, 1, 0, "Post-Lab: Distillation", "Lab (Minor)", at(0, 8), Status::InProgress, 10.0, false),
        item(
            Bucket::DueToday,
            2,
            1,
            "Read books 8 & 9 of The Odyssey; find 3 quotes",
            "Homework (Minor)",
            at(0, 10),
            Status::Completed,
            3.0,
            false,
        ),
        item(Bucket::DueTomorrow, 3, 2, "hw 5", "Homework (Minor)", at(1, 8), Status::Todo, 10.0, false),
        item(Bucket::DueTomorrow, 4, 4, "Map Analysis (1612)", "Homework (Minor)", at(1, 8), Status::Todo, 10.0, false),
        item(Bucket::DueTomorrow, 5, 1, "Socratic Seminar", "Class Participation (Minor)", at(1, 10), Status::InProgress, 3.0, false),
        task(900, "Study for the Health exam", at(1, 23), Bucket::DueTomorrow),
        item(Bucket::DueNextWeek, 6, 0, "Pre-Lab: Heat of Fusion", "Lab (Minor)", at(5, 8), Status::Todo, 2.0, false),
        item(Bucket::DueNextWeek, 7, 3, "Unit 3 Test", "Test/Exam (Major)", at(6, 8), Status::Todo, 50.0, true),
        item(Bucket::DueAfterNextWeek, 8, 4, "Periods 1 & 2 Assessment", "Test/Exam (Major)", at(14, 8), Status::Todo, 50.0, true),
        item(Bucket::PastThisWeek, 9, 2, "hw 4", "Homework (Minor)", at(-2, 8), Status::Completed, 10.0, false),
        item(Bucket::PastLastWeek, 10, 3, "TAREA", "Homework (Minor)", at(-9, 8), Status::Graded, 5.0, false),
    ];
    v[3].flags.late = true;
    v
}

pub fn detail(it: &CenterItem) -> AssignmentDetail {
    let dropbox = it.id == 4;
    AssignmentDetail {
        id: it.id,
        assignment_id: it.assignment_id,
        section_id: it.section_id,
        class: Some(it.class.clone()),
        name: it.name.clone(),
        assignment_type: it.assignment_type.clone(),
        assigned: it.assigned,
        due: it.due,
        description: Some(format!(
            "Complete the work for \"{}\". Show your reasoning, and bring questions to class.\n\nUse the attached handout; typed answers are fine.",
            it.name
        )),
        status: it.status,
        flags: it.flags,
        past_due: false,
        max_points: it.points,
        graded: it.status == Status::Graded,
        grade: (it.status == Status::Graded).then_some(4.5),
        percent: (it.status == Status::Graded).then_some(90.0),
        comment: (it.status == Status::Graded).then(|| "Nice work.".to_owned()),
        dropbox,
        downloads: vec![FileRef { name: "Handout.pdf".into(), path: "/ftpimages/602/download/download_1.pdf".into() }],
        submissions: if it.status == Status::Completed && it.id == 2 {
            vec![Submission {
                submitted: it.due,
                saved: it.due,
                ready: true,
                text: Some("1. The sea monsters suggest danger.\n2. Tourism boards.".into()),
            }]
        } else {
            vec![]
        },
    }
}

pub fn feed(now: NaiveDateTime) -> Vec<FeedItem> {
    vec![
        FeedItem {
            id: Some(2),
            ticks: Some(2),
            when: Some(now - Dur::hours(2)),
            from: Some("Mr. Brooks".into()),
            class: Some("ENGLISH III - 20".into()),
            assignment_id: Some(5),
            title: Some("Socratic Seminar".into()),
            text: Some("On Thursday and Friday we'll hold two seminars.\nBring your annotated text.".into()),
        },
        FeedItem {
            id: Some(1),
            ticks: Some(1),
            when: Some(now - Dur::hours(20)),
            from: Some("Ms. Alvarez".into()),
            class: Some("BIOLOGY - 20".into()),
            assignment_id: None,
            title: None,
            text: Some("Lab goggles are required from Monday.".into()),
        },
    ]
}

pub fn class_info(section_id: i64) -> ClassInfo {
    let (name, _, teacher) = CLASSES.iter().find(|c| c.1 == section_id).copied().unwrap_or(("?", 0, "?"));
    ClassInfo {
        name: name.into(),
        teacher: Some(teacher.into()),
        school_year: Some("2026 - 2027".into()),
        room: Some("404".into()),
        description: Some(
            "A year-long course covering the core ideas of the subject, with weekly labs and a comprehensive final exam.".into(),
        ),
        terms: vec![
            Term { name: Some("First Semester".into()), start: None, end: None },
            Term { name: Some("Second Semester".into()), start: None, end: None },
        ],
    }
}

pub fn class_rows(items: &[CenterItem], lead_section_id: i64) -> Vec<AssignmentRow> {
    items
        .iter()
        .filter(|i| i.section_id == lead_section_id && !i.is_task())
        .map(|i| AssignmentRow {
            id: i.id,
            name: i.name.clone(),
            assignment_type: i.assignment_type.clone(),
            due: i.due,
            status: i.status,
            major: i.major,
            has_grade: false,
            flags: i.flags,
        })
        .collect()
}

pub fn topics(now: NaiveDateTime) -> Vec<Topic> {
    vec![
        Topic { topic_id: 1, index_id: 11, name: "Unit 2: Reactions".into(), published: Some(now - Dur::days(12)) },
        Topic { topic_id: 2, index_id: 12, name: "Unit 1: Foundations".into(), published: Some(now - Dur::days(40)) },
    ]
}

pub fn topic_items(topic_id: i64) -> Vec<TopicItem> {
    vec![
        TopicItem {
            title: Some("Reading guide".into()),
            file: Some("Reading guide.pdf".into()),
            path: Some(format!("/ftpimages/602/download/topic_{topic_id}.pdf")),
            url: None,
            text: Some("Work through this before Friday.".into()),
        },
        TopicItem { title: Some("Simulation".into()), file: None, path: None, url: Some("https://phet.colorado.edu/".into()), text: None },
    ]
}

pub fn news(now: NaiveDateTime) -> Vec<Announcement> {
    vec![
        Announcement {
            title: "Room change".into(),
            author: Some("Ms. Alvarez".into()),
            published: Some(now - Dur::days(1)),
            text: Some("Thursday we meet in room 307 for the lab.".into()),
        },
        Announcement { title: "Welcome back".into(), author: None, published: Some(now - Dur::days(30)), text: None },
    ]
}

pub fn class_files(now: NaiveDateTime) -> Vec<ClassFile> {
    vec![
        ClassFile {
            name: "Syllabus.pdf".into(),
            path: "/ftpimages/602/download/syllabus.pdf".into(),
            published: Some(now - Dur::days(40)),
        },
        ClassFile {
            name: "Lab safety.docx".into(),
            path: "/ftpimages/602/download/safety.docx".into(),
            published: Some(now - Dur::days(39)),
        },
    ]
}

pub fn grades(section_id: i64) -> Result<Vec<Grades>, String> {
    if section_id == 104 {
        return Err(bromine_not_shared());
    }
    Ok(vec![Grades {
        period: "Quarter 1".into(),
        rows: vec![
            GradeRow::scored(1, "Lab 1", 9.0, 10.0),
            GradeRow::scored(2, "Quiz: Moles", 17.0, 20.0),
            GradeRow::scored(3, "hw 3", 3.0, 3.0),
        ],
    }])
}

fn bromine_not_shared() -> String {
    crate::worker::GRADES_NOT_SHARED.to_owned()
}

pub fn people() -> Vec<Person> {
    vec![
        Person { name: "Ms. Alvarez".into(), role: Some("Teacher".into()), grad_year: None, email: Some("alvarez@school.example".into()) },
        Person { name: "Ada Student".into(), role: Some("Student".into()), grad_year: Some("2028".into()), email: None },
        Person { name: "Grace Student".into(), role: Some("Student".into()), grad_year: Some("2028".into()), email: None },
    ]
}

pub fn user_files() -> Vec<UserFile> {
    vec![
        UserFile { name: "Fall schedule.pdf".into(), folder: Some("Schedules".into()), path: "/podium/UserFiles/Download.aspx?f=1".into() },
        UserFile { name: "Permission form.pdf".into(), folder: Some("Forms".into()), path: "/podium/UserFiles/Download.aspx?f=2".into() },
    ]
}

/// What the fake LMS answers for `key`.
pub fn payload(key: &Key, items: &[CenterItem], now: NaiveDateTime) -> Result<Payload, String> {
    Ok(match key {
        Key::Center => Payload::Center(items.to_vec()),
        Key::Classes => Payload::Classes(classes()),
        Key::Feed => Payload::Feed(feed(now)),
        Key::Detail(id) => {
            Payload::Detail(Box::new(items.iter().find(|i| !i.is_task() && i.id == *id).map(detail).ok_or("no such assignment")?))
        }
        Key::ClassInfo(section) => Payload::ClassInfo(Box::new(class_info(*section))),
        Key::ClassRows(lead) => Payload::ClassRows(class_rows(items, *lead)),
        Key::Topics(_) => Payload::Topics(topics(now)),
        Key::TopicItems { topic, .. } => Payload::TopicItems(topic_items(*topic)),
        Key::News(_) => Payload::News(news(now)),
        Key::ClassFiles(_) => Payload::ClassFiles(class_files(now)),
        Key::Grades(section) => Payload::Grades(grades(*section)?),
        Key::People(_) => Payload::People(people()),
        Key::UserFiles => Payload::UserFiles(user_files()),
    })
}

/// An in-memory LMS answering the UI's commands, with a small delay so spinners are visible.
pub fn spawn(rx: Receiver<Cmd>, tx: Sender<Event>, now: NaiveDateTime) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut items = items(now);
        let mut next_task = 901;
        while let Ok(cmd) = rx.recv() {
            std::thread::sleep(Duration::from_millis(150));
            let ev = match cmd {
                Cmd::Quit => break,
                Cmd::Load(key) => Event::Loaded { result: payload(&key, &items, now), key },
                Cmd::SetStatus { id, status, task } => {
                    let result = if status == Status::Completed && !task && id == 4 {
                        Err("'Map Analysis (1612)' takes a file submission and none has been made; it will be marked completed when you submit it in the LMS".to_owned())
                    } else {
                        items.iter_mut().filter(|i| i.id == id && i.is_task() == task).for_each(|i| i.status = status);
                        Ok(())
                    };
                    Event::StatusSet { id, status, task, result }
                }
                Cmd::CreateTask { description, due } => {
                    let id = next_task;
                    next_task += 1;
                    items.push(task_item(id, &description, due, now));
                    Event::TaskCreated(Ok(id))
                }
                Cmd::DeleteTask(id) => {
                    items.retain(|i| !(i.is_task() && i.id == id));
                    Event::TaskDeleted { id, result: Ok(()) }
                }
                Cmd::EditTask { id, patch } => {
                    edit_task(&mut items, id, &patch, now);
                    Event::TaskEdited(Ok(()))
                }
                // nothing is written in the demo
                Cmd::Fetch { name, action, .. } => Event::Fetched { result: Ok(PathBuf::from("~/Downloads").join(&name)), name, action },
            };
            if tx.send(ev).is_err() {
                break;
            }
        }
    })
}

fn edit_task(items: &mut [CenterItem], id: i64, patch: &TaskPatch, now: NaiveDateTime) {
    let linked =
        patch.assignment_index_id.and_then(|a| items.iter().find(|i| !i.is_task() && i.id == a).map(|i| (i.class.clone(), i.section_id)));
    if let Some(t) = items.iter_mut().find(|i| i.is_task() && i.id == id) {
        if let Some(d) = &patch.description {
            t.name = d.clone();
        }
        if let Some(due) = patch.due {
            *t = CenterItem { bucket: bucket_for(due, now), due: Some(due), ..t.clone() };
        }
        if let Some((class, section)) = linked {
            t.class = class;
            t.section_id = section;
        }
        if patch.unlink {
            t.class = "General Task".into();
            t.section_id = 0;
        }
    }
}

fn bucket_for(due: NaiveDateTime, now: NaiveDateTime) -> Bucket {
    match (due.date() - now.date()).num_days() {
        i64::MIN..=-1 => Bucket::Overdue,
        0 => Bucket::DueToday,
        1 => Bucket::DueTomorrow,
        2..=6 => Bucket::DueThisWeek,
        7..=13 => Bucket::DueNextWeek,
        _ => Bucket::DueAfterNextWeek,
    }
}

fn task_item(id: i64, name: &str, due: NaiveDateTime, now: NaiveDateTime) -> CenterItem {
    task(id, name, due, bucket_for(due, now))
}
