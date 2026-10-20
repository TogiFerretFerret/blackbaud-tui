// ==UserScript==
// @name         blackbaud-tui callback capture
// @namespace    https://github.com/TogiFerretFerret/blackbaud-tui
// @version      1.0
// @downloadURL  https://raw.githubusercontent.com/TogiFerretFerret/blackbaud-tui/main/capture.user.js
// @updateURL    https://raw.githubusercontent.com/TogiFerretFerret/blackbaud-tui/main/capture.user.js
// @match        https://sts.sky.blackbaud.com/azureadb2c/v2/callback/bbid*
// @grant        GM_xmlhttpRequest
// @connect      127.0.0.1
// @run-at       document-start
// ==/UserScript==

// the TUI registered the login attempt from Python, so this browser lacks the
// state cookie and the callback renders an error page instead of redirecting.
// That leaves the single-use `code` sitting in the URL for us to hand over.
GM_xmlhttpRequest({
    method: "POST",
    url: "http://127.0.0.1:8420/cb",
    data: location.href,
    onload: () => { document.title = "Captured - close this tab"; },
});
