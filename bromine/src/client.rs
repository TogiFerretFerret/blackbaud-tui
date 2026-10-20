//! The LMS web API, authenticated by a `t` cookie.

use std::sync::{Arc, Mutex};

use chrono::{Local, NaiveDateTime};
use reqwest::Method;
use reqwest::blocking::{Client as Http, Response};
use serde_json::{Value, json};
use url::Url;

use crate::auth::{self, Endpoints};
use crate::cookies::CookieBag;
use crate::error::{Error, Result};
use crate::json;
use crate::models::*;
use crate::text::{format_lms_datetime, html_to_text, parse_lms_datetime};

const NON_CLASS_CATEGORIES: &[&str] = &["External Program", "Academic Department Groups", "", "School Wide", "Faculty & Staff"];
const PERSONA_STUDENT: &str = "2";
/// Buckets whose status the assignment center gets wrong (observed: it says completed where the
/// assignment's own record says in progress or to do; every other bucket agreed).
const STALE_STATUS_BUCKETS: [Bucket; 3] = [Bucket::Missing, Bucket::Overdue, Bucket::DueToday];

/// A downloaded LMS file.
#[derive(Debug, Clone)]
pub struct Download {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Changes to a personal task. Fields left `None` keep their current value.
#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub description: Option<String>,
    pub due: Option<NaiveDateTime>,
    pub status: Option<Status>,
    pub section_id: Option<i64>,
    /// Link the task to this assignment (its index id) and so to its class, as the web UI does.
    pub assignment_index_id: Option<i64>,
    /// Drop any class/assignment link (the API clears links that an edit omits).
    pub unlink: bool,
}

/// A logged-in LMS session. Cheap to share between threads (`Arc<Client>`).
pub struct Client {
    ep: Endpoints,
    http: Http,
    cookies: Mutex<CookieBag>,
    context: Mutex<Option<Arc<Value>>>,
}

impl Client {
    pub fn new(tt: &str) -> Result<Self> {
        Self::with_endpoints(Endpoints::default(), tt)
    }

    pub fn with_endpoints(ep: Endpoints, tt: &str) -> Result<Self> {
        let mut cookies = CookieBag::new();
        cookies.set("t", tt);
        Ok(Self { ep, http: auth::http()?, cookies: Mutex::new(cookies), context: Mutex::new(None) })
    }

    // -------------------------------------------------------------- transport

    fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
        headers: &[(&str, String)],
    ) -> Result<Response> {
        let mut rb = self.http.request(method, format!("{}{path}", self.ep.lms));
        if !query.is_empty() {
            rb = rb.query(query);
        }
        if let Some(b) = body {
            rb = rb.json(b);
        }
        for (k, v) in headers {
            rb = rb.header(*k, v);
        }
        let resp = rb.header("cookie", self.cookies.lock().unwrap().header()).send()?;
        self.cookies.lock().unwrap().absorb(resp.headers());
        Ok(resp)
    }

    /// Does the LMS still accept our `t`? (The app's own status endpoint says so explicitly.)
    pub fn token_valid(&self) -> bool {
        self.send(Method::GET, "/api/webapp/userstatus", &[], None, &[])
            .ok()
            .filter(|r| r.status().is_success())
            .and_then(|r| r.json::<Value>().ok())
            .and_then(|v| v.get("TokenValid").and_then(Value::as_bool))
            .unwrap_or(false)
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
        headers: &[(&str, String)],
    ) -> Result<Response> {
        let resp = self.send(method.clone(), path, query, body, headers)?;
        let status = resp.status();
        // An expired `t` bounces to the login page, 401s, or gets a 403 "Request authorization failed"
        // (the same text as a genuine refusal, so ask the LMS whether the token is still good).
        if status.as_u16() == 401 || status.is_redirection() {
            return Err(Error::SessionExpired);
        }
        if status.as_u16() == 403 && !self.token_valid() {
            return Err(Error::SessionExpired);
        }
        if !status.is_success() {
            return Err(Error::Refused { status: status.as_u16(), method: method.to_string(), path: path.to_owned() });
        }
        Ok(resp)
    }

    fn json(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
        headers: &[(&str, String)],
    ) -> Result<Value> {
        let text = self.request(method, path, query, body, headers)?.text()?;
        // A 200 carrying an HTML login page also means "expired"; an empty body is fine (`{}`/`0` replies).
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|_| Error::SessionExpired)
    }

    fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value> {
        self.json(Method::GET, path, query, None, &[])
    }

    /// Mutating endpoints want the anti-forgery token echoed back as a header. The endpoint returns the
    /// string "<token> <token id>"; the header value is that verbatim.
    fn write_headers(&self) -> Result<Vec<(&'static str, String)>> {
        let token = self.get("/api/security/csrftoken", &[])?;
        let token = token.as_str().ok_or(Error::SessionExpired)?.to_owned();
        Ok(vec![("RequestVerificationToken", token), ("x-requested-with", "XMLHttpRequest".into())])
    }

    // -------------------------------------------------------------- identity & classes

    fn context(&self) -> Result<Arc<Value>> {
        if let Some(c) = self.context.lock().unwrap().clone() {
            return Ok(c);
        }
        let ctx = self.get("/api/webapp/context", &[])?;
        // With a dead `t` this endpoint still answers 200, with an anonymous context.
        if json::int(ctx.get("UserInfo").unwrap_or(&Value::Null), "UserId").is_none() {
            return Err(Error::SessionExpired);
        }
        let ctx = Arc::new(ctx);
        *self.context.lock().unwrap() = Some(ctx.clone());
        Ok(ctx)
    }

    pub fn user_id(&self) -> Result<i64> {
        json::int(self.context()?.get("UserInfo").unwrap_or(&Value::Null), "UserId").ok_or(Error::SessionExpired)
    }

    /// The student's current classes, one per lead section.
    pub fn classes(&self) -> Result<Vec<Class>> {
        let ctx = self.context()?;
        let mut seen = std::collections::HashSet::new();
        Ok(json::array(&ctx, "Groups")
            .iter()
            .filter(|g| !NON_CLASS_CATEGORIES.contains(&json::string(g, "Category").unwrap_or_default().as_str()))
            .filter_map(|g| {
                let lead = json::int(g, "LeadSectionId")?;
                seen.insert(lead).then(|| Class {
                    name: json::string(g, "GroupName").unwrap_or_default(),
                    section_id: json::int(g, "SectionId").unwrap_or(lead),
                    lead_section_id: lead,
                    teacher: json::string(g, "OwnerName"),
                    subject: json::string(g, "Category"),
                })
            })
            .collect())
    }

    pub fn class_info(&self, section_id: i64) -> Result<ClassInfo> {
        let v = self.get(
            "/api/datadirect/SectionInfoView/",
            &[("format", "json".into()), ("sectionId", section_id.to_string()), ("associationId", "1".into())],
        )?;
        let rows = v.as_array().map(Vec::as_slice).unwrap_or(&[]);
        ClassInfo::from_json(rows).ok_or(Error::Invalid(format!("no class info for section {section_id}")))
    }

    // -------------------------------------------------------------- assignments

    /// A class's assignments. `date_sort` is the web UI's value (0, 1 or 2); 1 is its default view.
    pub fn class_assignments(&self, lead_section_id: i64, date_sort: u8) -> Result<Vec<AssignmentRow>> {
        let v = self.get(
            &format!("/api/assignment/forsection/{lead_section_id}/"),
            &[("format", "json".into()), ("dateSort", date_sort.to_string()), ("personaId", PERSONA_STUDENT.into())],
        )?;
        Ok(v.as_array().map(|a| a.iter().filter_map(AssignmentRow::from_json).collect()).unwrap_or_default())
    }

    pub fn assignment(&self, assignment_index_id: i64) -> Result<AssignmentDetail> {
        let v = self.get(
            "/api/assignment2/UserAssignmentDetailsGetAllStudentData",
            &[
                ("assignmentIndexId", assignment_index_id.to_string()),
                ("studentUserId", self.user_id()?.to_string()),
                ("personaId", PERSONA_STUDENT.into()),
            ],
        )?;
        AssignmentDetail::from_json(&v).ok_or(Error::Invalid(format!("no such assignment: {assignment_index_id}")))
    }

    /// Set the student's own status on an assignment (visible to teachers).
    ///
    /// Only to do / in progress / completed can be set. "Completed" is refused for a file-submission
    /// assignment with no submission: this must never make unsubmitted work look turned in.
    pub fn set_status(&self, assignment_index_id: i64, status: Status) -> Result<()> {
        if !status.is_settable() {
            return Err(Error::Invalid(format!("{} is derived by the server and cannot be set", status.label())));
        }
        if status == Status::Completed {
            let detail = self.assignment(assignment_index_id)?;
            if detail.dropbox && detail.submissions.is_empty() {
                return Err(Error::Invalid(format!(
                    "'{}' takes a file submission and none has been made; it will be marked completed when you submit it in the LMS",
                    detail.name
                )));
            }
        }
        let headers = self.write_headers()?;
        self.json(
            Method::POST,
            "/api/assignment2/assignmentstatusupdate",
            &[],
            Some(&json!({"assignmentIndexId": assignment_index_id, "assignmentStatus": status.code()})),
            &headers,
        )?;
        Ok(())
    }

    // -------------------------------------------------------------- assignment center & tasks

    fn raw_center(&self) -> Result<Value> {
        self.get("/api/assignment2/StudentAssignmentCenterGet", &[("displayByDueDate", "true".into())])
    }

    /// Every assignment and personal task across all classes, in display order (bucket by bucket).
    ///
    /// The center's own status is wrong for the buckets in `STALE_STATUS_BUCKETS`, so for those the
    /// status is replaced with the assignment's own record (fetched concurrently).
    pub fn assignment_center(&self) -> Result<Vec<CenterItem>> {
        let raw = self.raw_center()?;
        let mut items: Vec<CenterItem> =
            Bucket::ALL.iter().flat_map(|&b| json::array(&raw, b.key()).iter().filter_map(move |v| CenterItem::from_json(b, v))).collect();

        let stale: Vec<usize> =
            items.iter().enumerate().filter(|(_, i)| !i.is_task() && STALE_STATUS_BUCKETS.contains(&i.bucket)).map(|(n, _)| n).collect();
        if !stale.is_empty() {
            self.user_id()?; // resolve once, before the worker threads need it
            let fixed: Vec<(usize, Option<Status>)> = std::thread::scope(|s| {
                let handles: Vec<_> = stale
                    .chunks(stale.len().div_ceil(6))
                    .map(|chunk| {
                        let items = &items;
                        s.spawn(move || chunk.iter().map(|&n| (n, self.assignment(items[n].id).ok().map(|d| d.status))).collect::<Vec<_>>())
                    })
                    .collect();
                handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
            });
            for (n, status) in fixed {
                if let Some(status) = status {
                    items[n].status = status;
                }
            }
        }
        Ok(items)
    }

    pub fn create_task(&self, description: &str, due: NaiveDateTime) -> Result<i64> {
        let description = description.trim();
        if description.is_empty() || description.chars().count() > 255 {
            return Err(Error::Invalid("a task description must be 1-255 characters".into()));
        }
        let user = self.user_id()?;
        let headers = self.write_headers()?;
        let id = self.json(
            Method::POST,
            "/api/UserTask/Edit/",
            &[],
            Some(&json!({
                "ShortDescription": description,
                "DueDate": format_lms_datetime(due),
                "AssignedDate": format_lms_datetime(Local::now().naive_local()),
                "TaskStatus": Status::Todo.code(),
                "UserId": user,
            })),
            &headers,
        )?;
        id.as_i64().ok_or_else(|| Error::Invalid("the LMS did not return a task id".into()))
    }

    fn personal_task(&self, raw: &Value, task_id: i64) -> Option<Value> {
        Bucket::ALL
            .iter()
            .flat_map(|b| json::array(raw, b.key()))
            .find(|v| json::flag(v, "UserTaskInd") && json::int(v, "UserTaskId") == Some(task_id))
            .cloned()
    }

    /// Change one of the student's personal tasks. Refuses ids that are not one.
    pub fn update_task(&self, task_id: i64, patch: &TaskPatch) -> Result<()> {
        if patch.unlink && (patch.section_id.is_some() || patch.assignment_index_id.is_some()) {
            return Err(Error::Invalid("unlink cannot be combined with a new link".into()));
        }
        if patch.status.is_some_and(|s| !s.is_settable()) {
            return Err(Error::Invalid("that status cannot be set on a task".into()));
        }
        let raw = self.raw_center()?;
        let cur =
            self.personal_task(&raw, task_id).ok_or_else(|| Error::Invalid(format!("{task_id} is not one of your personal tasks")))?;

        let mut task = json!({
            "UserTaskId": task_id,
            "ShortDescription": patch.description.clone().or_else(|| json::string(&cur, "ShortDescription")).unwrap_or_default(),
            "TaskStatus": patch.status.map(Status::code).or(json::int(&cur, "TaskStatus").map(|c| c as i32)).unwrap_or(Status::InProgress.code()),
            "DueDate": patch.due.map(format_lms_datetime).or_else(|| json::string(&cur, "DateDue")).unwrap_or_default(),
            "AssignedDate": json::string(&cur, "DateAssigned").unwrap_or_default(),
            "UserId": self.user_id()?,
        });
        let (mut section, mut assignment) =
            if patch.unlink { (None, None) } else { (json::int(&cur, "SectionId"), json::int(&cur, "AssignmentId")) };
        if let Some(aii) = patch.assignment_index_id {
            let target = Bucket::ALL
                .iter()
                .flat_map(|b| json::array(&raw, b.key()))
                .find(|v| !json::flag(v, "UserTaskInd") && json::int(v, "AssignmentIndexId") == Some(aii))
                .ok_or_else(|| Error::Invalid(format!("assignment {aii} is not in your assignment center")))?;
            section = json::int(target, "SectionId");
            assignment = json::int(target, "AssignmentId");
        }
        if let Some(s) = patch.section_id {
            section = Some(s);
        }
        for (key, value) in [("SectionId", section), ("AssignmentId", assignment)] {
            if let Some(v) = value.filter(|v| *v != 0) {
                task[key] = json!(v);
            }
        }
        let headers = self.write_headers()?;
        self.json(Method::PUT, "/api/UserTask/Edit/", &[], Some(&json!({ "userTask": task })), &headers)?;
        Ok(())
    }

    /// Delete one of the student's personal tasks. Refuses ids that are not one.
    pub fn delete_task(&self, task_id: i64) -> Result<()> {
        if self.personal_task(&self.raw_center()?, task_id).is_none() {
            return Err(Error::Invalid(format!("{task_id} is not one of your personal tasks")));
        }
        let headers = self.write_headers()?;
        self.json(Method::DELETE, "/api/UserTask/Edit", &[], Some(&json!({ "id": task_id })), &headers)?;
        Ok(())
    }

    // -------------------------------------------------------------- feed, content, grades

    pub fn activity_feed(&self) -> Result<Vec<FeedItem>> {
        let v = self.get("/api/datadirect/ActivityFeedGet/", &[("format", "json".into()), ("lastDate", String::new())])?;
        Ok(v.as_array().map(|a| a.iter().map(FeedItem::from_json).collect()).unwrap_or_default())
    }

    /// Posts newer than `ticks` (a [`FeedItem::ticks`]): one cheap call that is empty when nothing is new.
    pub fn activity_feed_since(&self, ticks: i64) -> Result<Vec<FeedItem>> {
        let v = self.get("/api/datadirect/ActivityFeedRefresh/", &[("format", "json".into()), ("firstDate", ticks.to_string())])?;
        Ok(v.as_array().map(|a| a.iter().map(FeedItem::from_json).collect()).unwrap_or_default())
    }

    pub fn topics(&self, section_id: i64) -> Result<Vec<Topic>> {
        let v = self.get(
            &format!("/api/datadirect/sectiontopicsget/{section_id}/"),
            &[
                ("format", "json".into()),
                ("active", "true".into()),
                ("future", "false".into()),
                ("expired", "false".into()),
                ("sharedTopics", "false".into()),
            ],
        )?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|t| {
                        Some(Topic {
                            topic_id: json::int(t, "TopicID")?,
                            index_id: json::int(t, "TopicIndexID")?,
                            name: json::string(t, "Name").and_then(|s| html_to_text(&s))?,
                            published: json::string(t, "PublishDate").and_then(|s| parse_lms_datetime(&s)),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn topic_items(&self, topic: &Topic) -> Result<Vec<TopicItem>> {
        let v = self.get(
            &format!("/api/datadirect/topiccontentget/{}/", topic.topic_id),
            &[("format", "json".into()), ("index_id", topic.index_id.to_string()), ("id", topic.index_id.to_string())],
        )?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .map(|i| {
                        let file = json::string(i, "FileName");
                        TopicItem {
                            title: json::string(i, "ShortDescription")
                                .or_else(|| json::string(i, "Headline"))
                                .and_then(|s| html_to_text(&s)),
                            file: json::string(i, "FriendlyFileName").or_else(|| file.clone()),
                            path: file.zip(json::string(i, "FilePath")).map(|(f, p)| format!("{p}{f}")),
                            url: json::string(i, "Url"),
                            text: json::string(i, "LongDescription").and_then(|s| html_to_text(&s)),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn announcements(&self, section_id: i64) -> Result<Vec<Announcement>> {
        let v = self.get(
            &format!("/api/announcement/forsection/{section_id}/"),
            &[
                ("format", "json".into()),
                ("editMode", "false".into()),
                ("active", "true".into()),
                ("future", "false".into()),
                ("expired", "false".into()),
                ("contextLabelId", "2".into()),
            ],
        )?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|n| {
                        Some(Announcement {
                            title: json::string(n, "Name").and_then(|s| html_to_text(&s))?,
                            author: json::string(n, "Author"),
                            published: json::string(n, "PublishDate").and_then(|s| parse_lms_datetime(&s)),
                            text: json::string(n, "Description").and_then(|s| html_to_text(&s)),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn class_files(&self, section_id: i64) -> Result<Vec<ClassFile>> {
        let v = self.get(
            &format!("/api/download/forsection/{section_id}/"),
            &[
                ("format", "json".into()),
                ("editMode", "false".into()),
                ("active", "false".into()),
                ("future", "false".into()),
                ("expired", "false".into()),
                ("contextLabelId", "2".into()),
            ],
        )?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|d| {
                        Some(ClassFile {
                            name: json::string(d, "Description")?,
                            path: json::string(d, "DownloadUrl")?,
                            published: json::string(d, "PublishDate").and_then(|s| parse_lms_datetime(&s)),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn roster(&self, section_id: i64) -> Result<Vec<Person>> {
        let v = self.get(&format!("/api/datadirect/sectionrosterget/{section_id}/"), &[("format", "json".into())])?;
        Ok(v.as_array().map(|a| a.iter().filter_map(Person::from_json).collect()).unwrap_or_default())
    }

    /// Files the school has published to this student (forms, account info, ...).
    pub fn user_files(&self) -> Result<Vec<UserFile>> {
        let v = self.get("/api/UserFile/UserFileGet/", &[])?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| {
                        Some(UserFile {
                            name: json::string(f, "Description")?,
                            folder: json::string(f, "FolderDescription"),
                            path: json::string(f, "DownloadLink")?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Per-assignment scores, one [`Grades`] per marking period. A class whose gradebook the teacher has
    /// not shared answers with [`Error::Refused`] (HTTP 403), not a session error.
    pub fn grades(&self, section_id: i64) -> Result<Vec<Grades>> {
        let periods = self.get("/api/datadirect/GradeBookMarkingPeriodList", &[("sectionId", section_id.to_string())])?;
        let student = self.user_id()?.to_string();
        let mut out = Vec::new();
        for p in periods.as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let Some(mp) = json::int(p, "MarkingPeriodId") else { continue };
            let rows = self.get(
                "/api/gradebook/AssignmentPerformanceStudent",
                &[("sectionId", section_id.to_string()), ("markingPeriodId", mp.to_string()), ("studentId", student.clone())],
            )?;
            out.push(Grades {
                period: json::string(p, "MarkingPeriodDescription").unwrap_or_else(|| mp.to_string()),
                rows: rows.as_array().map(|a| a.iter().filter_map(GradeRow::from_json).collect()).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    // -------------------------------------------------------------- files

    /// Normalise a file reference to a site-relative path, refusing anything that is not an LMS file
    /// download (this client must not become a general authenticated fetcher).
    pub fn file_path(&self, path: &str) -> Result<String> {
        let refuse = || Error::Invalid(format!("not an LMS file path: {path}"));
        let base = Url::parse(&self.ep.lms).map_err(|_| refuse())?;
        let parsed = if path.contains("://") { Url::parse(path).map_err(|_| refuse())? } else { base.join(path).map_err(|_| refuse())? };
        if parsed.host_str() != base.host_str()
            || parsed.scheme() != base.scheme()
            || parsed.port_or_known_default() != base.port_or_known_default()
        {
            return Err(refuse());
        }
        let p = parsed.path();
        if p.split('/').any(|s| s == "..") || !(p.starts_with("/ftpimages/") || p == "/podium/UserFiles/Download.aspx") {
            return Err(refuse());
        }
        Ok(match parsed.query() {
            Some(q) => format!("{p}?{q}"),
            None => p.to_owned(),
        })
    }

    pub fn download(&self, path: &str) -> Result<Download> {
        let path = self.file_path(path)?;
        let resp = self.request(Method::GET, &path, &[], None, &[])?;
        let name = resp
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .and_then(disposition_filename)
            .unwrap_or_else(|| path.split('?').next().and_then(|p| p.rsplit('/').next()).unwrap_or("download").to_owned());
        Ok(Download { name: sanitize_filename(&name), bytes: resp.bytes()?.to_vec() })
    }
}

fn disposition_filename(header: &str) -> Option<String> {
    let at = header.to_ascii_lowercase().find("filename=")? + "filename=".len();
    let rest = header[at..].trim_start();
    Some(match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?.to_owned(),
        None => rest.split(';').next()?.trim().to_owned(),
    })
}

/// A bare filename: no path separators or control characters, never hidden or empty.
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    if cleaned.is_empty() { "download".into() } else { cleaned.chars().take(150).collect() }
}
