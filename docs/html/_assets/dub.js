/*
 * Dub docs — shared interactions.
 *
 * Kept tiny, vanilla, zero-dependency, runs on a file:// URL. Each
 * feature only attaches if its DOM hook is present, so individual
 * pages can opt in.
 */
(function () {
    "use strict";

    /* --------------------------------------------------------------
     * Filterable table / kanban via [data-filter-target].
     *
     * Pattern:
     *   <input data-filter-target="#thing">
     *   <ul id="thing">
     *     <li data-filter-text="foo bar">…</li>
     *   </ul>
     *
     * On input, hides items whose data-filter-text doesn't include
     * the query (case-insensitive). Empty query shows all.
     * -------------------------------------------------------------- */
    document.querySelectorAll("[data-filter-target]").forEach(function (input) {
        var sel = input.getAttribute("data-filter-target");
        var container = document.querySelector(sel);
        if (!container) return;
        input.addEventListener("input", function () {
            var q = input.value.trim().toLowerCase();
            container.querySelectorAll("[data-filter-text]").forEach(function (row) {
                var hay = row.getAttribute("data-filter-text").toLowerCase();
                row.style.display = (q === "" || hay.indexOf(q) !== -1) ? "" : "none";
            });
            updateCounts(container);
        });
    });

    function updateCounts(container) {
        container.querySelectorAll("[data-count-of]").forEach(function (el) {
            var of = el.getAttribute("data-count-of");
            var visible = container.querySelectorAll(of + ":not([style*='display: none'])").length;
            el.textContent = String(visible);
        });
    }

    /* --------------------------------------------------------------
     * Tag-based filter chips.
     *
     *   <button data-filter-tag="bug">Bugs</button>
     *   <li data-tags="bug urgent">…</li>
     *
     * Multiple chips active = union. "all" chip clears the filter.
     * -------------------------------------------------------------- */
    var activeTags = new Set();
    document.querySelectorAll("[data-filter-tag]").forEach(function (chip) {
        chip.addEventListener("click", function () {
            var tag = chip.getAttribute("data-filter-tag");
            if (tag === "all") {
                activeTags.clear();
                document.querySelectorAll("[data-filter-tag]").forEach(function (c) {
                    c.classList.remove("is-active");
                });
                chip.classList.add("is-active");
            } else {
                chip.classList.toggle("is-active");
                if (chip.classList.contains("is-active")) activeTags.add(tag);
                else activeTags.delete(tag);
                var allChip = document.querySelector("[data-filter-tag='all']");
                if (allChip) allChip.classList.toggle("is-active", activeTags.size === 0);
            }
            document.querySelectorAll("[data-tags]").forEach(function (item) {
                var have = item.getAttribute("data-tags").split(/\s+/);
                var show = activeTags.size === 0 || have.some(function (t) { return activeTags.has(t); });
                item.style.display = show ? "" : "none";
            });
        });
    });

    /* --------------------------------------------------------------
     * Schema table picker.
     *
     * Sidebar with [data-schema-table="tracks"] entries links to
     * <section data-schema-pane="tracks">. Clicking a link reveals
     * the pane and scrolls it into view.
     * -------------------------------------------------------------- */
    document.querySelectorAll("[data-schema-table]").forEach(function (link) {
        link.addEventListener("click", function (e) {
            e.preventDefault();
            var name = link.getAttribute("data-schema-table");
            document.querySelectorAll("[data-schema-pane]").forEach(function (pane) {
                pane.classList.toggle("is-active", pane.getAttribute("data-schema-pane") === name);
            });
            document.querySelectorAll("[data-schema-table]").forEach(function (l) {
                l.classList.toggle("is-active", l === link);
            });
            var target = document.querySelector("[data-schema-pane='" + name + "']");
            if (target) target.scrollIntoView({ behavior: "smooth", block: "start" });
        });
    });

    /* --------------------------------------------------------------
     * Schema diagram → pane focus.
     *
     * SVG tables in the ER diagram are <g data-schema-jump="tracks">.
     * Clicking jumps the pane the same way the sidebar does.
     * -------------------------------------------------------------- */
    document.querySelectorAll("[data-schema-jump]").forEach(function (node) {
        node.style.cursor = "pointer";
        node.addEventListener("click", function () {
            var name = node.getAttribute("data-schema-jump");
            var link = document.querySelector("[data-schema-table='" + name + "']");
            if (link) link.click();
        });
    });

    /* --------------------------------------------------------------
     * Active-page highlight in the nav.
     * -------------------------------------------------------------- */
    var path = location.pathname.split("/").pop() || "index.html";
    document.querySelectorAll(".dub-nav__links a").forEach(function (a) {
        var href = a.getAttribute("href");
        if (href === path) a.classList.add("is-active");
    });

    /* --------------------------------------------------------------
     * Keyboard shortcut: '/' focuses the first filter input on the
     * page (matches the muscle memory the LibraryView shortcut sheet
     * eventually wants per UI-BACKLOG U-23).
     * -------------------------------------------------------------- */
    document.addEventListener("keydown", function (e) {
        if (e.key === "/" && document.activeElement.tagName !== "INPUT") {
            var firstFilter = document.querySelector("[data-filter-target]");
            if (firstFilter) {
                e.preventDefault();
                firstFilter.focus();
            }
        }
    });
}());
