//! What the library hands back: small typed structs, already cleaned (HTML stripped, dates parsed,
//! sentinels turned into `None`).

use chrono::NaiveDateTime;
use serde_json::Value;

use crate::json;
use crate::text::{html_to_text, parse_lms_datetime};

// ------------------------------------------------------------------ status & buckets

/// A student's progress on an assignment or task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Todo,
    InProgress,
    Completed,
    /// Derived by the server; cannot be set from here.
    Overdue,
    /// Derived by the server; cannot be set from here.
    Graded,
    Other(i32),
}

impl Status {
    /// The three states a student may set.
    pub const SETTABLE: [Status; 3] = [Status::Todo, Status::InProgress, Status::Completed];

    pub fn from_code(code: i64) -> Self {
        match code {
            -1 => Status::Todo,
            0 => Status::InProgress,
            1 => Status::Completed,
            2 => Status::Overdue,
            4 => Status::Graded,
            other => Status::Other(other as i32),
        }
    }

    pub fn code(self) -> i32 {
        match self {
            Status::Todo => -1,
            Status::InProgress => 0,
            Status::Completed => 1,
            Status::Overdue => 2,
            Status::Graded => 4,
            Status::Other(c) => c,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Todo => "to do",
            Status::InProgress => "in progress",
            Status::Completed => "completed",
            Status::Overdue => "overdue",
            Status::Graded => "graded",
            Status::Other(_) => "?",
        }
    }

    pub fn is_settable(self) -> bool {
        Self::SETTABLE.contains(&self)
    }

    /// Nothing left for the student to do.
    pub fn is_done(self) -> bool {
        matches!(self, Status::Completed | Status::Graded)
    }
}

/// The assignment center's own grouping, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Bucket {
    Missing,
    Overdue,
    DueToday,
    DueTomorrow,
    DueThisWeek,
    DueNextWeek,
    DueAfterNextWeek,
    PastThisWeek,
    PastLastWeek,
    PastBeforeLastWeek,
}

impl Bucket {
    pub const ALL: [Bucket; 10] = [
        Bucket::Missing,
        Bucket::Overdue,
        Bucket::DueToday,
        Bucket::DueTomorrow,
        Bucket::DueThisWeek,
        Bucket::DueNextWeek,
        Bucket::DueAfterNextWeek,
        Bucket::PastThisWeek,
        Bucket::PastLastWeek,
        Bucket::PastBeforeLastWeek,
    ];

    /// The key in the LMS's JSON.
    pub fn key(self) -> &'static str {
        match self {
            Bucket::Missing => "Missing",
            Bucket::Overdue => "Overdue",
            Bucket::DueToday => "DueToday",
            Bucket::DueTomorrow => "DueTomorrow",
            Bucket::DueThisWeek => "DueThisWeek",
            Bucket::DueNextWeek => "DueNextWeek",
            Bucket::DueAfterNextWeek => "DueAfterNextWeek",
            Bucket::PastThisWeek => "PastThisWeek",
            Bucket::PastLastWeek => "PastLastWeek",
            Bucket::PastBeforeLastWeek => "PastBeforeLastWeek",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Bucket::Missing => "Missing",
            Bucket::Overdue => "Overdue",
            Bucket::DueToday => "Due today",
            Bucket::DueTomorrow => "Due tomorrow",
            Bucket::DueThisWeek => "Due this week",
            Bucket::DueNextWeek => "Due next week",
            Bucket::DueAfterNextWeek => "Due later",
            Bucket::PastThisWeek => "Earlier this week",
            Bucket::PastLastWeek => "Last week",
            Bucket::PastBeforeLastWeek => "Before last week",
        }
    }

    pub fn is_upcoming(self) -> bool {
        self <= Bucket::DueAfterNextWeek
    }
}

// ------------------------------------------------------------------ assignment center

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Assignment,
    /// One of the student's own "My tasks" to-dos.
    Task,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    pub late: bool,
    pub missing: bool,
    pub exempt: bool,
    pub incomplete: bool,
}

impl Flags {
    pub(crate) fn from_json(v: &Value, suffix: &str) -> Self {
        let f = |name: &str| json::flag(v, &format!("{name}{suffix}"));
        Flags { late: f("Late"), missing: f("Missing"), exempt: f("Exempt"), incomplete: f("Incomplete") }
    }

    pub fn is_empty(self) -> bool {
        self == Flags::default()
    }

    pub fn describe(self) -> String {
        [(self.late, "late"), (self.missing, "missing"), (self.exempt, "exempt"), (self.incomplete, "incomplete")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// One row of the assignment center.
#[derive(Debug, Clone, PartialEq)]
pub struct CenterItem {
    pub bucket: Bucket,
    pub kind: ItemKind,
    /// `AssignmentIndexId` for assignments; the task id for tasks.
    pub id: i64,
    /// `AssignmentId` (not the index id); needed to link a task to an assignment.
    pub assignment_id: i64,
    pub section_id: i64,
    pub class: String,
    pub name: String,
    pub assignment_type: Option<String>,
    pub due: Option<NaiveDateTime>,
    pub assigned: Option<NaiveDateTime>,
    pub status: Status,
    pub points: Option<f64>,
    pub major: bool,
    pub flags: Flags,
    pub has_download: bool,
}

impl CenterItem {
    pub(crate) fn from_json(bucket: Bucket, v: &Value) -> Option<Self> {
        let is_task = json::flag(v, "UserTaskInd");
        let id = if is_task { json::int(v, "UserTaskId")? } else { json::int(v, "AssignmentIndexId")? };
        // The center's own status is wrong for items due today (see `Client::assignment_center`); tasks
        // carry theirs in `TaskStatus`.
        let status_code =
            if is_task { json::int(v, "TaskStatus").unwrap_or(0) } else { json::int(v, "AssignmentStatusType").unwrap_or(-1) };
        Some(CenterItem {
            bucket,
            kind: if is_task { ItemKind::Task } else { ItemKind::Assignment },
            id,
            assignment_id: json::int(v, "AssignmentId").unwrap_or(0),
            section_id: json::int(v, "SectionId").unwrap_or(0),
            class: json::string(v, "GroupName").unwrap_or_default(),
            name: json::string(v, "ShortDescription").and_then(|s| html_to_text(&s)).unwrap_or_else(|| "(untitled)".into()),
            assignment_type: json::string(v, "AssignmentType"),
            due: json::string(v, "DateDue").and_then(|s| parse_lms_datetime(&s)),
            assigned: json::string(v, "DateAssigned").and_then(|s| parse_lms_datetime(&s)),
            status: Status::from_code(status_code),
            points: json::float(v, "MaxPoints").filter(|p| *p > 0.0),
            major: json::flag(v, "Major"),
            flags: Flags::from_json(v, "Ind"),
            has_download: json::flag(v, "HasDownload"),
        })
    }

    pub fn is_task(&self) -> bool {
        self.kind == ItemKind::Task
    }
}

// ------------------------------------------------------------------ classes

#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: String,
    pub section_id: i64,
    pub lead_section_id: i64,
    pub teacher: Option<String>,
    pub subject: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Term {
    pub name: Option<String>,
    pub start: Option<NaiveDateTime>,
    pub end: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassInfo {
    pub name: String,
    pub teacher: Option<String>,
    pub school_year: Option<String>,
    pub room: Option<String>,
    pub description: Option<String>,
    pub terms: Vec<Term>,
}

impl ClassInfo {
    pub(crate) fn from_json(sections: &[Value]) -> Option<Self> {
        let first = sections.first()?;
        let group = json::string(first, "GroupName")?;
        Some(ClassInfo {
            name: match json::string(first, "Identifier") {
                Some(id) => format!("{group} - {id}"),
                None => group,
            },
            teacher: json::string(first, "Teacher"),
            school_year: json::string(first, "SchoolYear"),
            room: json::string(first, "Room"),
            description: json::string(first, "Description").and_then(|s| html_to_text(&s)),
            terms: sections
                .iter()
                .map(|s| Term {
                    name: json::string(s, "Duration"),
                    start: json::string(s, "StartDate").and_then(|d| parse_lms_datetime(&d)),
                    end: json::string(s, "EndDate").and_then(|d| parse_lms_datetime(&d)),
                })
                .collect(),
        })
    }
}

// ------------------------------------------------------------------ assignments

/// One row of a class's assignment list.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentRow {
    pub id: i64,
    pub name: String,
    pub assignment_type: Option<String>,
    pub due: Option<NaiveDateTime>,
    pub status: Status,
    pub major: bool,
    pub has_grade: bool,
    pub flags: Flags,
}

impl AssignmentRow {
    pub(crate) fn from_json(v: &Value) -> Option<Self> {
        Some(AssignmentRow {
            id: json::int(v, "AssignmentIndexId")?,
            name: json::string(v, "AssignmentDescription").and_then(|s| html_to_text(&s)).unwrap_or_else(|| "(untitled)".into()),
            assignment_type: json::string(v, "AssignmentType"),
            due: json::string(v, "DateDue").and_then(|s| parse_lms_datetime(&s)),
            status: Status::from_code(json::int(v, "AssignmentStatus").unwrap_or(-1)),
            major: json::flag(v, "Major"),
            has_grade: json::flag(v, "HasGrade"),
            flags: Flags::from_json(v, "Ind"),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileRef {
    pub name: String,
    /// A site-relative path for [`crate::Client::download`].
    pub path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Submission {
    pub submitted: Option<NaiveDateTime>,
    pub saved: Option<NaiveDateTime>,
    pub ready: bool,
    /// Typed text, if the student typed their answer in the LMS.
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentDetail {
    pub id: i64,
    pub assignment_id: i64,
    pub section_id: i64,
    pub class: Option<String>,
    pub name: String,
    pub assignment_type: Option<String>,
    pub assigned: Option<NaiveDateTime>,
    pub due: Option<NaiveDateTime>,
    pub description: Option<String>,
    pub status: Status,
    pub flags: Flags,
    pub past_due: bool,
    pub max_points: Option<f64>,
    pub graded: bool,
    pub grade: Option<f64>,
    pub percent: Option<f64>,
    pub comment: Option<String>,
    /// Takes a file submission through the LMS.
    pub dropbox: bool,
    pub downloads: Vec<FileRef>,
    pub submissions: Vec<Submission>,
}

impl AssignmentDetail {
    pub(crate) fn from_json(v: &Value) -> Option<Self> {
        let grade_block = v.get("AssignmentGrade").filter(|g| g.is_object()).cloned().unwrap_or(Value::Null);
        let max_points = json::float(v, "MaxPoints").filter(|p| *p > 0.0);
        let grade = json::float(&grade_block, "GradebookGrade");
        let graded = json::flag(v, "PublishGrade") && grade.is_some();
        Some(AssignmentDetail {
            id: json::int(v, "AssignmentIndexId")?,
            assignment_id: json::int(v, "AssignmentId").unwrap_or(0),
            section_id: json::int(v, "SectionId").unwrap_or(0),
            class: json::string(v, "GroupName"),
            name: json::string(v, "ShortDescription").and_then(|s| html_to_text(&s)).unwrap_or_else(|| "(untitled)".into()),
            assignment_type: json::string(v, "AssignmentType"),
            assigned: json::string(v, "AssignmentDate").and_then(|s| parse_lms_datetime(&s)),
            due: json::string(v, "DueDate").and_then(|s| parse_lms_datetime(&s)),
            description: json::string(v, "LongDescription").and_then(|s| html_to_text(&s)),
            status: Status::from_code(json::int(&grade_block, "AssignmentStatusType").unwrap_or(-1)),
            flags: Flags::from_json(&grade_block, "Ind"),
            past_due: json::flag(v, "PastDue"),
            max_points,
            graded,
            grade: grade.filter(|_| graded),
            percent: match (graded, grade, max_points) {
                (true, Some(g), Some(m)) => Some(g / m * 100.0),
                _ => None,
            },
            comment: json::string(&grade_block, "GradedComment").and_then(|s| html_to_text(&s)),
            dropbox: json::flag(v, "DropboxInd"),
            downloads: json::array(v, "DownloadItems")
                .iter()
                .filter_map(|d| {
                    Some(FileRef {
                        name: json::string(d, "FriendlyFileName").or_else(|| json::string(d, "ShortDescription"))?,
                        path: json::string(d, "DownloadUrl")?,
                    })
                })
                .collect(),
            submissions: json::array(v, "SubmissionResults")
                .iter()
                .map(|r| Submission {
                    submitted: json::string(r, "LastSubmitDate").and_then(|s| parse_lms_datetime(&s)),
                    saved: json::string(r, "LastSavedDate").and_then(|s| parse_lms_datetime(&s)),
                    ready: json::flag(r, "ReadyInd"),
                    text: json::string(r, "Detail").and_then(|s| html_to_text(&s)),
                })
                .collect(),
        })
    }
}

// ------------------------------------------------------------------ class content, feed, grades

#[derive(Debug, Clone, PartialEq)]
pub struct Topic {
    pub topic_id: i64,
    pub index_id: i64,
    pub name: String,
    pub published: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TopicItem {
    pub title: Option<String>,
    pub file: Option<String>,
    pub path: Option<String>,
    pub url: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Announcement {
    pub title: String,
    pub author: Option<String>,
    pub published: Option<NaiveDateTime>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassFile {
    pub name: String,
    pub path: String,
    pub published: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeedItem {
    pub when: Option<NaiveDateTime>,
    pub from: Option<String>,
    pub class: Option<String>,
    /// The `AssignmentIndexId`, when the post is an assignment.
    pub assignment_id: Option<i64>,
    pub title: Option<String>,
    pub text: Option<String>,
}

impl FeedItem {
    pub(crate) fn from_json(v: &Value) -> Self {
        // "CLASS NAME $^sectionId $^assocId $^assignmentIndexId" (last part only on assignment posts)
        let loc: Vec<&str> = v.get("content_locations").and_then(Value::as_str).unwrap_or("").split("$^").map(str::trim).collect();
        FeedItem {
            when: json::string(v, "publish_date").or_else(|| json::string(v, "audit_date")).and_then(|s| parse_lms_datetime(&s)),
            from: json::string(v, "user_name"),
            class: loc.first().filter(|s| !s.is_empty()).map(|s| s.to_string()),
            assignment_id: (json::int(v, "additional_assignment_detail") == Some(1))
                .then(|| loc.get(3).and_then(|s| s.parse().ok()))
                .flatten(),
            title: json::string(v, "content_short_description").and_then(|s| html_to_text(&s)),
            text: json::string(v, "content_long_description").and_then(|s| html_to_text(&s)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GradeRow {
    pub id: i64,
    pub name: String,
    pub assignment_type: Option<String>,
    pub due: Option<NaiveDateTime>,
    pub points: Option<f64>,
    pub max_points: Option<f64>,
    pub flags: Flags,
    pub comment: Option<String>,
    counted: bool,
}

impl GradeRow {
    /// A scored row that counts towards the total (for tests and demos; the API builds rows with `from_json`).
    pub fn scored(id: i64, name: &str, points: f64, max_points: f64) -> Self {
        GradeRow {
            id,
            name: name.into(),
            assignment_type: None,
            due: None,
            points: Some(points),
            max_points: Some(max_points),
            flags: Flags::default(),
            comment: None,
            counted: true,
        }
    }

    pub(crate) fn from_json(v: &Value) -> Option<Self> {
        let points = json::float(v, "Points");
        let exempt = json::flag(v, "Exempt");
        Some(GradeRow {
            id: json::int(v, "AssignmentIndexId")?,
            name: json::string(v, "AssignmentShortDescription")
                .or_else(|| json::string(v, "Assignment"))
                .and_then(|s| html_to_text(&s))
                .unwrap_or_default(),
            assignment_type: json::string(v, "AssignmentType"),
            due: json::string(v, "DateDue").and_then(|s| parse_lms_datetime(&s)),
            points: points.filter(|_| !exempt),
            max_points: json::float(v, "MaxPoints"),
            flags: Flags::from_json(v, ""),
            comment: json::string(v, "Comment").and_then(|s| html_to_text(&s)),
            counted: points.is_some() && !exempt && v.get("IncludeCumulativeGrade").and_then(Value::as_bool).unwrap_or(true),
        })
    }
}

/// Per-assignment scores for one marking period. The school hides cumulative course grades from
/// students, so the total is plain points earned / points possible and is **unweighted**.
#[derive(Debug, Clone, PartialEq)]
pub struct Grades {
    pub period: String,
    pub rows: Vec<GradeRow>,
}

impl Grades {
    pub fn unweighted_total(&self) -> Option<(f64, f64)> {
        let counted = self.rows.iter().filter(|r| r.counted);
        let (earned, possible) = counted.fold((0.0, 0.0), |(e, p), r| (e + r.points.unwrap_or(0.0), p + r.max_points.unwrap_or(0.0)));
        (possible > 0.0).then_some((earned, possible))
    }

    pub fn unweighted_percent(&self) -> Option<f64> {
        self.unweighted_total().map(|(e, p)| e / p * 100.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserFile {
    pub name: String,
    pub folder: Option<String>,
    pub path: String,
}

/// One person on a class roster. Only teachers expose an email address.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub name: String,
    pub role: Option<String>,
    pub grad_year: Option<String>,
    pub email: Option<String>,
}

impl Person {
    pub(crate) fn from_json(v: &Value) -> Option<Self> {
        let teacher = json::string(v, "teacherType");
        Some(Person {
            name: json::string(v, "name")?,
            email: json::string(v, "email").filter(|_| teacher.is_some()),
            role: teacher.or_else(|| json::string(v, "enrollmentType")),
            grad_year: json::string(v, "gradYear"),
        })
    }
}
