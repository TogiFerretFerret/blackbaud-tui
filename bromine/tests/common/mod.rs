//! Three mock servers standing in for sts.sky, sts-sso and the LMS, plus fixtures shaped like real captures.
#![allow(dead_code)]

use bromine::{Client, Endpoints};
use mockito::{Matcher, Mock, Server, ServerGuard};
use serde_json::{Value, json};

pub struct Lms {
    pub sts: ServerGuard,
    pub sso: ServerGuard,
    pub lms: ServerGuard,
}

/// A path with or without a query string.
pub fn path(p: &str) -> Matcher {
    Matcher::Regex(format!(r"^{}/?(\?.*)?$", regex_escape(p.trim_end_matches('/'))))
}

fn regex_escape(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '/' || c == '_' || c == '-' { c.to_string() } else { format!("\\{c}") })
        .collect()
}

impl Lms {
    pub fn new() -> Self {
        Self { sts: Server::new(), sso: Server::new(), lms: Server::new() }
    }

    pub fn endpoints(&self) -> Endpoints {
        Endpoints {
            sts: self.sts.url(),
            sts_sso: self.sso.url(),
            lms: self.lms.url(),
            b2c_authorize: format!("{}/authorize", self.sts.url()),
        }
    }

    pub fn client(&self) -> Client {
        Client::with_endpoints(self.endpoints(), "TT").unwrap()
    }

    /// GET `p` on the LMS answers `body` as JSON.
    pub fn get(&mut self, p: &str, body: Value) -> Mock {
        self.lms.mock("GET", path(p)).with_header("content-type", "application/json").with_body(body.to_string()).create()
    }

    /// The identity endpoints every authenticated call leans on.
    pub fn signed_in(&mut self) {
        self.get("/api/webapp/userstatus", json!({"TokenValid": true}));
        self.get("/api/webapp/context", context());
    }

    pub fn csrf(&mut self) -> Mock {
        self.lms
            .mock("GET", path("/api/security/csrftoken"))
            .with_header("content-type", "application/json")
            .with_header("set-cookie", "__RequestVerificationToken_OnSuite=cookietoken; path=/; secure")
            .with_body("\"TOK TOKID\"")
            .create()
    }
}

pub fn context() -> Value {
    let class = |name: &str, section: i64, lead: i64, category: &str| json!({"GroupName": name, "SectionId": section, "LeadSectionId": lead, "Category": category, "OwnerName": "Ms. Teacher"});
    json!({
        "UserInfo": {"UserId": 7},
        "Groups": [
            class("CHEMISTRY - 20", 10, 10, "Science"),
            class("CHEMISTRY - 20", 11, 10, "Science"),      // duplicate lead section: dropped
            class("School Wide", 12, 12, "School Wide"),     // not a class: dropped
            class("MATH-IV E - 10", 20, 20, "Mathematics"),
        ],
    })
}

/// An assignment-center item as the API sends it.
pub fn center_item(aii: i64, name: &str, group: &str, due: &str, status_type: i64) -> Value {
    json!({
        "GroupName": group, "SectionId": 10, "AssignmentId": aii + 1000, "AssignmentIndexId": aii,
        "ShortDescription": name, "AssignmentType": "Homework (Minor)", "DateDue": due, "DateAssigned": "9/29/2026 12:00 AM",
        "AssignmentStatusType": status_type, "MaxPoints": 10, "Major": 0, "UserTaskInd": 0, "UserTaskId": 0,
        "LateInd": 0, "MissingInd": 0, "ExemptInd": 0, "IncompleteInd": 0, "HasDownload": 0,
    })
}

pub fn task_item(id: i64, name: &str, status: i64) -> Value {
    json!({
        "GroupName": "General Task", "SectionId": 0, "AssignmentId": 0, "AssignmentIndexId": 0, "ShortDescription": name,
        "AssignmentType": "My tasks", "DateDue": "10/3/2026 11:59 PM", "DateAssigned": "9/30/2026 1:08 PM",
        "AssignmentStatusType": 1, "TaskStatus": status, "UserTaskInd": 1, "UserTaskId": id,
    })
}

/// `UserAssignmentDetailsGetAllStudentData`.
pub fn detail(aii: i64, status_type: i64, dropbox: bool, submissions: Value) -> Value {
    json!({
        "AssignmentIndexId": aii, "AssignmentId": aii + 1000, "SectionId": 10, "GroupName": "CHEMISTRY - 20",
        "ShortDescription": "Pre-Lab: Heat of Fusion", "AssignmentType": "Lab (Minor)",
        "AssignmentDate": "9/29/2026 12:00 AM", "DueDate": "10/5/2026 8:00 AM",
        "LongDescription": "<p>Do the <b>reading</b> &amp; answer</p>", "MaxPoints": 2.0, "PublishGrade": true,
        "DropboxInd": dropbox, "PastDue": false,
        "DownloadItems": [{"FriendlyFileName": "Handout.pdf", "DownloadUrl": "/ftpimages/602/download/download_1.pdf"}],
        "SubmissionResults": submissions,
        "AssignmentGrade": {"AssignmentStatusType": status_type, "GradebookGrade": null},
    })
}
