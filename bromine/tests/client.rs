mod common;

use bromine::{Bucket, Client, Error, ItemKind, Status, TaskPatch};
use chrono::NaiveDate;
use common::{Lms, center_item, detail, path, task_item};
use mockito::Matcher;
use serde_json::json;

fn due(y: i32, m: u32, d: u32, h: u32, min: u32) -> chrono::NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
}

// ------------------------------------------------------------------ expiry detection

#[test]
fn a_403_means_expired_only_when_the_lms_says_the_token_is_invalid() {
    let mut s = Lms::new();
    let body = json!({"Error": "Request authorization failed.  Check your username & password and try to get a new token."});
    s.lms.mock("GET", path("/api/assignment2/StudentAssignmentCenterGet")).with_status(403).with_body(body.to_string()).create();
    s.get("/api/webapp/userstatus", json!({"TokenValid": false}));
    assert!(matches!(s.client().assignment_center(), Err(Error::SessionExpired)));

    let mut s = Lms::new();
    s.lms.mock("GET", path("/api/assignment2/StudentAssignmentCenterGet")).with_status(403).with_body(body.to_string()).create();
    s.get("/api/webapp/userstatus", json!({"TokenValid": true}));
    assert!(
        matches!(s.client().assignment_center(), Err(Error::Refused { status: 403, .. })),
        "a genuine refusal must not look like expiry"
    );
}

#[test]
fn redirects_401_html_and_anonymous_context_all_count_as_expired() {
    let mut s = Lms::new();
    s.lms.mock("GET", path("/api/webapp/context")).with_status(302).with_header("location", "/app").create();
    assert!(matches!(s.client().classes(), Err(Error::SessionExpired)));

    let mut s = Lms::new();
    s.lms.mock("GET", path("/api/webapp/context")).with_status(401).create();
    assert!(matches!(s.client().classes(), Err(Error::SessionExpired)));

    let mut s = Lms::new();
    s.lms.mock("GET", path("/api/webapp/context")).with_body("<html>login</html>").create();
    assert!(matches!(s.client().classes(), Err(Error::SessionExpired)));

    let mut s = Lms::new();
    s.get("/api/webapp/context", json!({"UserInfo": {"StudentDisplay": "x"}, "Tasks": []}));
    assert!(matches!(s.client().classes(), Err(Error::SessionExpired)), "a dead `t` still gets a 200 with an anonymous context");
}

#[test]
fn the_session_cookie_is_sent_and_later_cookies_are_echoed() {
    let mut s = Lms::new();
    s.signed_in();
    let _csrf = s.csrf();
    let post = s
        .lms
        .mock("POST", "/api/assignment2/assignmentstatusupdate")
        .match_header("cookie", Matcher::Regex(r"t=TT".into()))
        .match_header("cookie", Matcher::Regex(r"__RequestVerificationToken_OnSuite=cookietoken".into()))
        .match_header("RequestVerificationToken", "TOK TOKID")
        .match_body(Matcher::PartialJson(json!({"assignmentIndexId": 5, "assignmentStatus": 0})))
        .with_body(r#"{"WasVoid":true}"#)
        .create();
    s.client().set_status(5, Status::InProgress).unwrap();
    post.assert();
}

// ------------------------------------------------------------------ classes, assignments, center

#[test]
fn classes_are_deduped_and_non_classes_dropped() {
    let mut s = Lms::new();
    s.signed_in();
    let classes = s.client().classes().unwrap();
    assert_eq!(classes.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["CHEMISTRY - 20", "MATH-IV E - 10"]);
    assert_eq!(classes[0].lead_section_id, 10);
    assert_eq!(classes[0].teacher.as_deref(), Some("Ms. Teacher"));
}

#[test]
fn the_center_status_for_items_due_today_comes_from_the_assignments_own_record() {
    let mut s = Lms::new();
    s.signed_in();
    s.get(
        "/api/assignment2/StudentAssignmentCenterGet",
        json!({
            "DueToday": [center_item(1, "Post-Lab", "CHEM", "9/30/2026 8:00 AM", 1), center_item(2, "Quiz &amp; more", "ENG", "9/30/2026 11:00 AM", 1)],
            "DueTomorrow": [center_item(3, "hw 5", "MATH", "10/1/2026 8:00 AM", -1)],
            "PastLastWeek": [center_item(4, "old", "MATH", "9/20/2026 8:00 AM", 1)],
            "Sections": [1, 2],
        }),
    );
    // item 1 is really in progress (0); item 2 really is completed (1)
    let d1 = s
        .lms
        .mock("GET", path("/api/assignment2/UserAssignmentDetailsGetAllStudentData"))
        .match_query(Matcher::UrlEncoded("assignmentIndexId".into(), "1".into()))
        .with_body(detail(1, 0, false, json!([])).to_string())
        .expect(1)
        .create();
    let d2 = s
        .lms
        .mock("GET", path("/api/assignment2/UserAssignmentDetailsGetAllStudentData"))
        .match_query(Matcher::UrlEncoded("assignmentIndexId".into(), "2".into()))
        .with_body(detail(2, 1, false, json!([])).to_string())
        .expect(1)
        .create();

    let items = s.client().assignment_center().unwrap();
    d1.assert();
    d2.assert(); // and no detail requests for the other buckets
    let by_id = |id| items.iter().find(|i| i.id == id).unwrap();
    assert_eq!(by_id(1).status, Status::InProgress, "center said completed");
    assert_eq!(by_id(2).status, Status::Completed);
    assert_eq!(by_id(3).status, Status::Todo);
    assert_eq!(by_id(2).name, "Quiz & more");
    assert_eq!(by_id(4).bucket, Bucket::PastLastWeek);
    assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), [1, 2, 3, 4], "bucket order is display order");
    assert_eq!(by_id(1).due, Some(due(2026, 9, 30, 8, 0)));
    assert_eq!(by_id(1).points, Some(10.0));
}

#[test]
fn tasks_show_up_in_the_center_with_their_own_id_and_status() {
    let mut s = Lms::new();
    s.signed_in();
    s.get("/api/assignment2/StudentAssignmentCenterGet", json!({"DueTomorrow": [task_item(1456381, "Study", -1)]}));
    let items = s.client().assignment_center().unwrap();
    assert_eq!(items[0].kind, ItemKind::Task);
    assert_eq!((items[0].id, items[0].status, items[0].class.as_str()), (1456381, Status::Todo, "General Task"));
}

#[test]
fn assignment_details_include_files_submission_text_and_clean_html() {
    let mut s = Lms::new();
    s.signed_in();
    let subs = json!([{"LastSubmitDate": "9/30/2026 2:29 PM", "LastSavedDate": "9/30/2026 2:29 PM", "ReadyInd": true,
                       "Detail": "<div>1. Answer&#160;<br />two &amp; three</div>", "DownloadUrl": "/app/file/DropBoxFile?uTolken=SECRET"}]);
    s.get("/api/assignment2/UserAssignmentDetailsGetAllStudentData", detail(42, 1, true, subs));
    let d = s.client().assignment(42).unwrap();
    assert_eq!(d.description.as_deref(), Some("Do the reading & answer"));
    assert_eq!(d.status, Status::Completed);
    assert!(d.dropbox);
    assert_eq!(d.downloads[0].path, "/ftpimages/602/download/download_1.pdf");
    assert_eq!(d.submissions[0].text.as_deref(), Some("1. Answer\ntwo & three"));
    assert_eq!(d.submissions[0].submitted, Some(due(2026, 9, 30, 14, 29)));
    assert!(!format!("{d:?}").contains("SECRET"), "the submission's token-bearing link is not carried");
}

#[test]
fn completed_is_refused_for_unsubmitted_file_assignments_and_nothing_is_written() {
    let mut s = Lms::new();
    s.signed_in();
    s.get("/api/assignment2/UserAssignmentDetailsGetAllStudentData", detail(5, -1, true, json!([])));
    let post = s.lms.mock("POST", "/api/assignment2/assignmentstatusupdate").expect(0).create();
    let err = s.client().set_status(5, Status::Completed).unwrap_err();
    assert!(err.to_string().contains("takes a file submission and none has been made"), "{err}");
    post.assert();
}

#[test]
fn completed_is_allowed_with_a_submission_or_without_a_file_assignment_and_derived_states_never() {
    for (dropbox, subs) in [(true, json!([{"ReadyInd": true}])), (false, json!([]))] {
        let mut s = Lms::new();
        s.signed_in();
        let _csrf = s.csrf();
        s.get("/api/assignment2/UserAssignmentDetailsGetAllStudentData", detail(5, -1, dropbox, subs));
        let post = s.lms.mock("POST", "/api/assignment2/assignmentstatusupdate").with_body("{}").expect(1).create();
        s.client().set_status(5, Status::Completed).unwrap();
        post.assert();
    }
    let s = Lms::new();
    assert!(matches!(s.client().set_status(5, Status::Graded), Err(Error::Invalid(_))));
    assert!(matches!(s.client().set_status(5, Status::Overdue), Err(Error::Invalid(_))));
}

// ------------------------------------------------------------------ tasks

#[test]
fn create_update_link_unlink_and_delete_a_task() {
    let mut s = Lms::new();
    s.signed_in();
    let _csrf = s.csrf();
    let mut center =
        json!({"DueToday": [center_item(41, "Pre-Lab", "CHEM", "10/5/2026 8:00 AM", -1), task_item(900, "buy poster board", -1)]});
    s.get("/api/assignment2/StudentAssignmentCenterGet", center.clone());

    let create = s
        .lms
        .mock("POST", "/api/UserTask/Edit/")
        .match_body(Matcher::PartialJson(
            json!({"ShortDescription": "study", "DueDate": "10/2/2026 11:59 PM", "TaskStatus": -1, "UserId": 7}),
        ))
        .with_body("1456316")
        .create();
    let client = s.client();
    assert_eq!(client.create_task("  study  ", due(2026, 10, 2, 23, 59)).unwrap(), 1456316);
    create.assert();
    assert!(client.create_task("", due(2026, 10, 2, 23, 59)).is_err());
    assert!(client.create_task(&"x".repeat(256), due(2026, 10, 2, 23, 59)).is_err());

    // status only: everything else (including the original assigned date) is preserved
    let mut put = |body: serde_json::Value| {
        s.lms.mock("PUT", "/api/UserTask/Edit/").match_body(Matcher::PartialJson(body)).with_body("{}").expect(1).create()
    };
    let m = put(json!({"userTask": {"UserTaskId": 900, "ShortDescription": "buy poster board", "TaskStatus": 1,
                                    "DueDate": "10/3/2026 11:59 PM", "AssignedDate": "9/30/2026 1:08 PM", "UserId": 7}}));
    client.update_task(900, &TaskPatch { status: Some(Status::Completed), ..Default::default() }).unwrap();
    m.assert();

    // link to an assignment: the class and AssignmentId (not the index id) come from the center
    let m =
        put(json!({"userTask": {"ShortDescription": "renamed", "DueDate": "10/3/2026 8:00 AM", "SectionId": 10, "AssignmentId": 1041}}));
    client
        .update_task(
            900,
            &TaskPatch {
                description: Some("renamed".into()),
                due: Some(due(2026, 10, 3, 8, 0)),
                assignment_index_id: Some(41),
                ..Default::default()
            },
        )
        .unwrap();
    m.assert();

    assert!(client.update_task(900, &TaskPatch { assignment_index_id: Some(999), ..Default::default() }).is_err());
    assert!(
        client.update_task(41, &TaskPatch { status: Some(Status::Todo), ..Default::default() }).is_err(),
        "an assignment id is not a task"
    );
    assert!(client.update_task(900, &TaskPatch { status: Some(Status::Graded), ..Default::default() }).is_err());
    assert!(client.update_task(900, &TaskPatch { unlink: true, section_id: Some(5), ..Default::default() }).is_err());

    // unlink: a task that is linked now sends neither field
    center["DueToday"][1]["SectionId"] = json!(10);
    center["DueToday"][1]["AssignmentId"] = json!(1041);
    s.lms.reset();
    s.signed_in();
    let _csrf = s.csrf();
    s.get("/api/assignment2/StudentAssignmentCenterGet", center);
    let body = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let seen = body.clone();
    s.lms
        .mock("PUT", "/api/UserTask/Edit/")
        .with_body_from_request(move |r| {
            *seen.lock().unwrap() = String::from_utf8_lossy(r.body().unwrap()).into();
            b"{}".to_vec()
        })
        .create();
    let client = s.client();
    client.update_task(900, &TaskPatch { unlink: true, ..Default::default() }).unwrap();
    let sent = body.lock().unwrap().clone();
    assert!(sent.contains("buy poster board") && !sent.contains("SectionId") && !sent.contains("AssignmentId"), "{sent}");
}

#[test]
fn delete_refuses_anything_that_is_not_a_personal_task() {
    let mut s = Lms::new();
    s.signed_in();
    let _csrf = s.csrf();
    s.get(
        "/api/assignment2/StudentAssignmentCenterGet",
        json!({"DueToday": [center_item(41, "Pre-Lab", "CHEM", "10/5/2026 8:00 AM", -1), task_item(900, "t", -1)]}),
    );
    let del =
        s.lms.mock("DELETE", "/api/UserTask/Edit").match_body(Matcher::PartialJson(json!({"id": 900}))).with_body("0").expect(1).create();
    let client = s.client();
    assert!(client.delete_task(41).is_err(), "an assignment id");
    client.delete_task(900).unwrap();
    del.assert();
}

// ------------------------------------------------------------------ content, grades, files

#[test]
fn grades_unweighted_total_skips_ungraded_exempt_and_uncounted_rows() {
    let mut s = Lms::new();
    s.signed_in();
    s.get("/api/datadirect/GradeBookMarkingPeriodList", json!([{"MarkingPeriodId": 20659, "MarkingPeriodDescription": "First Semester"}]));
    s.get(
        "/api/gradebook/AssignmentPerformanceStudent",
        json!([
            {"AssignmentIndexId": 1, "AssignmentShortDescription": "A", "Points": 8.5, "MaxPoints": 10.0},
            {"AssignmentIndexId": 2, "AssignmentShortDescription": "B", "Points": -2147483648, "MaxPoints": 10.0},
            {"AssignmentIndexId": 3, "AssignmentShortDescription": "C", "Points": 5.0, "MaxPoints": 10.0, "Exempt": true},
            {"AssignmentIndexId": 4, "AssignmentShortDescription": "D", "Points": 1.5, "MaxPoints": 5.0, "IncludeCumulativeGrade": false},
        ]),
    );
    let grades = s.client().grades(10).unwrap();
    assert_eq!(grades[0].period, "First Semester");
    assert_eq!(grades[0].unweighted_total(), Some((8.5, 10.0)));
    assert_eq!(grades[0].unweighted_percent(), Some(85.0));
    assert_eq!(grades[0].rows.iter().map(|r| r.points).collect::<Vec<_>>(), [Some(8.5), None, None, Some(1.5)]);
}

#[test]
fn an_unshared_gradebook_is_a_refusal_not_an_expired_session() {
    let mut s = Lms::new();
    s.signed_in();
    s.get("/api/datadirect/GradeBookMarkingPeriodList", json!([{"MarkingPeriodId": 1}]));
    s.lms
        .mock("GET", path("/api/gradebook/AssignmentPerformanceStudent"))
        .with_status(403)
        .with_body("{\"Error\":\"Request authorization failed.\"}")
        .create();
    assert!(matches!(s.client().grades(10), Err(Error::Refused { status: 403, .. })));
}

#[test]
fn feed_topics_announcements_and_files_parse() {
    let mut s = Lms::new();
    s.signed_in();
    s.get(
        "/api/datadirect/ActivityFeedGet",
        json!([
        {"publish_date": "9/30/2026 11:00 AM", "user_name": "T", "content_short_description": "Sem", "content_long_description": "a<br />b",
         "additional_assignment_detail": 1, "content_locations": "ENGLISH III - 20 $^28858390 $^1 $^42016874"},
        {"user_name": "U", "content_locations": "", "content_short_description": "news"}]),
    );
    let feed = s.client().activity_feed().unwrap();
    assert_eq!(
        (feed[0].assignment_id, feed[0].class.as_deref(), feed[0].text.as_deref()),
        (Some(42016874), Some("ENGLISH III - 20"), Some("a\nb"))
    );
    assert_eq!((feed[1].assignment_id, feed[1].class.clone()), (None, None));

    s.get(
        "/api/datadirect/sectiontopicsget/10",
        json!([{"TopicID": 5, "TopicIndexID": 6, "Name": "Unit 1", "PublishDate": "9/9/2026 12:00 AM"}]),
    );
    s.get(
        "/api/datadirect/topiccontentget/5",
        json!([
        {"ShortDescription": "Lab", "FriendlyFileName": "Lab.docx", "FileName": "download_1.docx", "FilePath": "/ftpimages/602/download/"},
        {"Headline": "Site", "Url": "https://x.org"}]),
    );
    let client = s.client();
    let topics = client.topics(10).unwrap();
    assert_eq!((topics[0].topic_id, topics[0].index_id), (5, 6));
    let items = client.topic_items(&topics[0]).unwrap();
    assert_eq!(items[0].path.as_deref(), Some("/ftpimages/602/download/download_1.docx"));
    assert_eq!((items[1].title.as_deref(), items[1].url.as_deref()), (Some("Site"), Some("https://x.org")));

    s.get(
        "/api/announcement/forsection/10",
        json!([{"Name": "Rooms", "Author": "", "PublishDate": "9/9/2026 12:00 AM", "Description": "M: 307<br>Tu: 422"}]),
    );
    let ann = client.announcements(10).unwrap();
    assert_eq!((ann[0].author.clone(), ann[0].text.as_deref()), (None, Some("M: 307\nTu: 422")));
}

#[test]
fn the_roster_hides_student_emails() {
    let mut s = Lms::new();
    s.signed_in();
    s.get(
        "/api/datadirect/sectionrosterget/10",
        json!([
            {"name": "Ms. Teacher", "teacherType": "Teacher", "email": "t@school.org"},
            {"name": "A Student", "enrollmentType": "Student", "gradYear": "2028", "email": "kid@x.org"},
            {"email": "nameless@x.org"}]),
    );
    let people = s.client().roster(10).unwrap();
    assert_eq!(people.len(), 2, "a row without a name is dropped");
    assert_eq!((people[0].role.as_deref(), people[0].email.as_deref()), (Some("Teacher"), Some("t@school.org")));
    assert_eq!((people[1].role.as_deref(), people[1].grad_year.as_deref(), people[1].email.clone()), (Some("Student"), Some("2028"), None));
}

#[test]
fn only_lms_files_can_be_downloaded_and_names_are_taken_from_the_server_safely() {
    let mut s = Lms::new();
    let client: Client = s.client();
    let host = s.lms.url();
    assert_eq!(client.file_path("/ftpimages/602/download/a.pdf").unwrap(), "/ftpimages/602/download/a.pdf");
    assert_eq!(client.file_path(&format!("{host}/ftpimages/602/a.pdf")).unwrap(), "/ftpimages/602/a.pdf");
    let q = "/podium/UserFiles/Download.aspx?s=a%2bb&f=c";
    assert_eq!(client.file_path(q).unwrap(), q);
    for bad in [
        "https://evil.example/ftpimages/a",
        "/api/webapp/context",
        "/ftpimages/../api/x",
        "/podium/UserFiles/Other.aspx",
        "http://127.0.0.1:1/ftpimages/a",
    ] {
        assert!(client.file_path(bad).is_err(), "{bad}");
    }

    s.lms
        .mock("GET", "/ftpimages/602/download/download_1.pdf")
        .with_header("content-disposition", "attachment; filename=\"../../etc/passwd\"")
        .with_body("PDFDATA")
        .create();
    let dl = client.download("/ftpimages/602/download/download_1.pdf").unwrap();
    assert_eq!((dl.name.as_str(), dl.bytes.as_slice()), (".._.._etc_passwd".trim_start_matches('.'), b"PDFDATA".as_slice()));
    assert!(!dl.name.contains('/'));
    s.lms.mock("GET", "/ftpimages/602/download/plain.docx").with_body("x").create();
    assert_eq!(client.download("/ftpimages/602/download/plain.docx").unwrap().name, "plain.docx");
    assert_eq!(bromine::sanitize_filename("  ..hidden "), "hidden");
    assert_eq!(bromine::sanitize_filename("///"), "___");
    assert_eq!(bromine::sanitize_filename(""), "download");
}
