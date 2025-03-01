"use strict";
var details, searchbtn, unzoombtn, matchedtxt, svg, searching, frames, known_font_width;
function init(evt) {
    details = document.getElementById("details").firstChild;
    searchbtn = document.getElementById("search");
    unzoombtn = document.getElementById("unzoom");
    matchedtxt = document.getElementById("matched");
    svg = document.getElementsByTagName("svg")[0];
    frames = document.getElementById("frames");
    known_font_width = get_monospace_width(frames);
    total_samples = parseInt(frames.attributes.total_samples.value);
    searching = 0;

    // Use GET parameters to restore a flamegraph's state.
    var restore_state = function() {
        var params = get_params();
        if (params.x && params.y)
            zoom(find_group(document.querySelector('[*|x="' + params.x + '"][y="' + params.y + '"]')));
        if (params.s)
            search(params.s);
    };

    if (fluiddrawing) {
        // Make width dynamic so the SVG fits its parent's width.
        svg.removeAttribute("width");
        // Edge requires us to have a viewBox that gets updated with size changes.
        var isEdge = /Edge\/\d./i.test(navigator.userAgent);
        if (!isEdge) {
            svg.removeAttribute("viewBox");
        }
        var update_for_width_change = function() {
            if (isEdge) {
                svg.attributes.viewBox.value = "0 0 " + svg.width.baseVal.value + " " + svg.height.baseVal.value;
            }

            // Keep consistent padding on left and right of frames container.
            frames.attributes.width.value = svg.width.baseVal.value - xpad * 2;

            // Text truncation needs to be adjusted for the current width.
            update_text_for_elements(frames.children);

            // Keep search elements at a fixed distance from right edge.
            var svgWidth = svg.width.baseVal.value;
            searchbtn.attributes.x.value = svgWidth - xpad;
            matchedtxt.attributes.x.value = svgWidth - xpad;
        };
        window.addEventListener('resize', function() {
            update_for_width_change();
        });
        // This needs to be done asynchronously for Safari to work.
        setTimeout(function() {
            unzoom();
            update_for_width_change();
            restore_state();
        }, 0);
    } else {
        restore_state();
    }
}
// event listeners
window.addEventListener("click", function(e) {
    var target = find_group(e.target);
    if (target) {
        if (target.nodeName == "a") {
            if (e.ctrlKey === false) return;
            e.preventDefault();
        }
        if (target.classList.contains("parent")) unzoom();
        zoom(target);

        // set parameters for zoom state
        var el = target.querySelector("rect");
        if (el && el.attributes && el.attributes.y && el.attributes["fg:x"]) {
            var params = get_params()
            params.x = el.attributes["fg:x"].value;
            params.y = el.attributes.y.value;
            history.replaceState(null, null, parse_params(params));
        }
    }
    else if (e.target.id == "unzoom") {
        unzoom();

        // remove zoom state
        var params = get_params();
        if (params.x) delete params.x;
        if (params.y) delete params.y;
        history.replaceState(null, null, parse_params(params));
    }
    else if (e.target.id == "search") search_prompt();
}, false)
// mouse-over for info
// show
window.addEventListener("mouseover", function(e) {
    var target = find_group(e.target);
    if (target) details.nodeValue = nametype + " " + g_to_text(target);
}, false)
// clear
window.addEventListener("mouseout", function(e) {
    var target = find_group(e.target);
    if (target) details.nodeValue = ' ';
}, false)
// ctrl-F for search
window.addEventListener("keydown",function (e) {
    if (e.keyCode === 114 || (e.ctrlKey && e.keyCode === 70)) {
        e.preventDefault();
        search_prompt();
    }
}, false)
// functions
function get_params() {
    var params = {};
    var paramsarr = window.location.search.substr(1).split('&');
    for (var i = 0; i < paramsarr.length; ++i) {
        var tmp = paramsarr[i].split("=");
        if (!tmp[0] || !tmp[1]) continue;
        params[tmp[0]]  = decodeURIComponent(tmp[1]);
    }
    return params;
}
function parse_params(params) {
    var uri = "?";
    for (var key in params) {
        uri += key + '=' + encodeURIComponent(params[key]) + '&';
    }
    if (uri.slice(-1) == "&")
        uri = uri.substring(0, uri.length - 1);
    if (uri == '?')
        uri = window.location.href.split('?')[0];
    return uri;
}
function find_child(node, selector) {
    var children = node.querySelectorAll(selector);
    if (children.length) return children[0];
    return;
}
function find_group(node) {
    var parent = node.parentElement;
    if (!parent) return;
    if (parent.id == "frames") return node;
    return find_group(parent);
}
function orig_save(e, attr, val) {
    if (e.attributes["fg:orig_" + attr] != undefined) return;
    if (e.attributes[attr] == undefined) return;
    if (val == undefined) val = e.attributes[attr].value;
    e.setAttribute("fg:orig_" + attr, val);
}
function orig_load(e, attr) {
    if (e.attributes["fg:orig_"+attr] == undefined) return;
    e.attributes[attr].value = e.attributes["fg:orig_" + attr].value;
    e.removeAttribute("fg:orig_" + attr);
}
function g_to_text(e) {
    var text = find_child(e, "title").firstChild.nodeValue;
    return (text)
}
function g_to_func(e) {
    var func = g_to_text(e);
    // if there's any manipulation we want to do to the function
    // name before it's searched, do it here before returning.
    return (func);
}
function get_monospace_width(frames) {
    // Given the id="frames" element, return the width of text characters if
    // this is a monospace font, otherwise return 0.
    text = find_child(frames.children[0], "text");
    originalContent = text.textContent;
    text.textContent = "!";
    bangWidth = text.getComputedTextLength();
    text.textContent = "W";
    wWidth = text.getComputedTextLength();
    text.textContent = originalContent;
    if (bangWidth === wWidth) {
        return bangWidth;
    } else {
        return 0;
    }
}
function update_text_for_elements(elements) {
    // In order to render quickly in the browser, you want to do one pass of
    // reading attributes, and one pass of mutating attributes. See
    // https://web.dev/avoid-large-complex-layouts-and-layout-thrashing/ for details.

    // Fall back to inefficient calculation, if we're variable-width font.
    // TODO This should be optimized somehow too.
    if (known_font_width === 0) {
        for (var i = 0; i < elements.length; i++) {
            update_text(elements[i]);
        }
        return;
    }

    var textElemNewAttributes = [];
    for (var i = 0; i < elements.length; i++) {
        var e = elements[i];
        var r = find_child(e, "rect");
        var t = find_child(e, "text");
        var w = parseFloat(r.attributes.width.value) * frames.attributes.width.value / 100 - 3;
        var txt = find_child(e, "title").textContent.replace(/\([^(]*\)$/,"");
        var newX = format_percent((parseFloat(r.attributes.x.value) + (100 * 3 / frames.attributes.width.value)));

        // Smaller than this size won't fit anything
        if (w < 2 * known_font_width) {
            textElemNewAttributes.push([newX, ""]);
            continue;
        }

        // Fit in full text width
        if (txt.length * known_font_width < w) {
            textElemNewAttributes.push([newX, txt]);
            continue;
        }

        var substringLength = Math.floor(w / known_font_width) - 2;
        if (truncate_text_right) {
            // Truncate the right side of the text.
            textElemNewAttributes.push([newX, txt.substring(0, substringLength) + ".."]);
            continue;
        } else {
            // Truncate the left side of the text.
            textElemNewAttributes.push([newX, ".." + txt.substring(txt.length - substringLength, txt.length)]);
            continue;
        }
    }

    console.assert(textElemNewAttributes.length === elements.length, "Resize failed, please file a bug at https://github.com/jonhoo/inferno/");

    // Now that we know new textContent, set it all in one go so we don't refresh a bazillion times.
    for (var i = 0; i < elements.length; i++) {
        var e = elements[i];
        var values = textElemNewAttributes[i];
        var t = find_child(e, "text");
        t.attributes.x.value = values[0];
        t.textContent = values[1];
    }
}

function update_text(e) {
    var r = find_child(e, "rect");
    var t = find_child(e, "text");
    var w = parseFloat(r.attributes.width.value) * frames.attributes.width.value / 100 - 3;
    var txt = find_child(e, "title").textContent.replace(/\([^(]*\)$/,"");
    t.attributes.x.value = format_percent((parseFloat(r.attributes.x.value) + (100 * 3 / frames.attributes.width.value)));

    // Smaller than this size won't fit anything
    if (w < 2 * fontsize * fontwidth) {
        t.textContent = "";
        return;
    }
    t.textContent = txt;
    // Fit in full text width
    if (t.getComputedTextLength() < w)
        return;
    if (truncate_text_right) {
        // Truncate the right side of the text.
        for (var x = txt.length - 2; x > 0; x--) {
            if (t.getSubStringLength(0, x + 2) <= w) {
                t.textContent = txt.substring(0, x) + "..";
                return;
            }
        }
    } else {
        // Truncate the left side of the text.
        for (var x = 2; x < txt.length; x++) {
            if (t.getSubStringLength(x - 2, txt.length) <= w) {
                t.textContent = ".." + txt.substring(x, txt.length);
                return;
            }
        }
    }
    t.textContent = "";
}
// zoom
function zoom_reset(e) {
    if (e.tagName == "rect") {
        e.attributes.x.value = format_percent(100 * parseInt(e.attributes["fg:x"].value) / total_samples);
        e.attributes.width.value = format_percent(100 * parseInt(e.attributes["fg:w"].value) / total_samples);
    }
    if (e.childNodes == undefined) return;
    for(var i = 0, c = e.childNodes; i < c.length; i++) {
        zoom_reset(c[i]);
    }
}
function zoom_child(e, x, zoomed_width_samples) {
    if (e.tagName == "text") {
        var parent_x = parseFloat(find_child(e.parentNode, "rect[x]").attributes.x.value);
        e.attributes.x.value = format_percent(parent_x + (100 * 3 / frames.attributes.width.value));
    } else if (e.tagName == "rect") {
        e.attributes.x.value = format_percent(100 * (parseInt(e.attributes["fg:x"].value) - x) / zoomed_width_samples);
        e.attributes.width.value = format_percent(100 * parseInt(e.attributes["fg:w"].value) / zoomed_width_samples);
    }
    if (e.childNodes == undefined) return;
    for(var i = 0, c = e.childNodes; i < c.length; i++) {
        zoom_child(c[i], x, zoomed_width_samples);
    }
}
function zoom_parent(e) {
    if (e.attributes) {
        if (e.attributes.x != undefined) {
            e.attributes.x.value = "0.0%";
        }
        if (e.attributes.width != undefined) {
            e.attributes.width.value = "100.0%";
        }
    }
    if (e.childNodes == undefined) return;
    for(var i = 0, c = e.childNodes; i < c.length; i++) {
        zoom_parent(c[i]);
    }
}
function zoom(node) {
    var attr = find_child(node, "rect").attributes;
    var width = parseInt(attr["fg:w"].value);
    var xmin = parseInt(attr["fg:x"].value);
    var xmax = xmin + width;
    var ymin = parseFloat(attr.y.value);
    unzoombtn.classList.remove("hide");
    var el = frames.children;
    var to_update_text = [];
    for (var i = 0; i < el.length; i++) {
        var e = el[i];
        var a = find_child(e, "rect").attributes;
        var ex = parseInt(a["fg:x"].value);
        var ew = parseInt(a["fg:w"].value);
        // Is it an ancestor
        if (!inverted) {
            var upstack = parseFloat(a.y.value) > ymin;
        } else {
            var upstack = parseFloat(a.y.value) < ymin;
        }
        if (upstack) {
            // Direct ancestor
            if (ex <= xmin && (ex+ew) >= xmax) {
                e.classList.add("parent");
                zoom_parent(e);
                to_update_text.push(e);
            }
            // not in current path
            else
                e.classList.add("hide");
        }
        // Children maybe
        else {
            // no common path
            if (ex < xmin || ex >= xmax) {
                e.classList.add("hide");
            }
            else {
                zoom_child(e, xmin, width);
                to_update_text.push(e);
            }
        }
    }
    update_text_for_elements(to_update_text);
}
function unzoom() {
    unzoombtn.classList.add("hide");
    var el = frames.children;
    for(var i = 0; i < el.length; i++) {
        el[i].classList.remove("parent");
        el[i].classList.remove("hide");
        zoom_reset(el[i]);
    }
    update_text_for_elements(el);
}
// search
function reset_search() {
    var el = document.querySelectorAll("#frames rect");
    for (var i = 0; i < el.length; i++) {
        orig_load(el[i], "fill")
    }
    var params = get_params();
    delete params.s;
    history.replaceState(null, null, parse_params(params));
}
function search_prompt() {
    if (!searching) {
        var term = prompt("Enter a search term (regexp " +
            "allowed, eg: ^ext4_)", "");
        if (term != null) {
            search(term)
        }
    } else {
        reset_search();
        searching = 0;
        searchbtn.classList.remove("show");
        searchbtn.firstChild.nodeValue = "Search"
        matchedtxt.classList.add("hide");
        matchedtxt.firstChild.nodeValue = ""
    }
}
function search(term) {
    var re = new RegExp(term);
    var el = frames.children;
    var matches = new Object();
    var maxwidth = 0;
    for (var i = 0; i < el.length; i++) {
        var e = el[i];
        // Skip over frames which are either not visible, or below the zoomed-to frame
        if (e.classList.contains("hide") || e.classList.contains("parent")) {
            continue;
        }
        var func = g_to_func(e);
        var rect = find_child(e, "rect");
        if (func == null || rect == null)
            continue;
        // Save max width. Only works as we have a root frame
        var w = parseInt(rect.attributes["fg:w"].value);
        if (w > maxwidth)
            maxwidth = w;
        if (func.match(re)) {
            // highlight
            var x = parseInt(rect.attributes["fg:x"].value);
            orig_save(rect, "fill");
            rect.attributes.fill.value = searchcolor;
            // remember matches
            if (matches[x] == undefined) {
                matches[x] = w;
            } else {
                if (w > matches[x]) {
                    // overwrite with parent
                    matches[x] = w;
                }
            }
            searching = 1;
        }
    }
    if (!searching)
        return;
    var params = get_params();
    params.s = term;
    history.replaceState(null, null, parse_params(params));

    searchbtn.classList.add("show");
    searchbtn.firstChild.nodeValue = "Reset Search";
    // calculate percent matched, excluding vertical overlap
    var count = 0;
    var lastx = -1;
    var lastw = 0;
    var keys = Array();
    for (k in matches) {
        if (matches.hasOwnProperty(k))
            keys.push(k);
    }
    // sort the matched frames by their x location
    // ascending, then width descending
    keys.sort(function(a, b){
        return a - b;
    });
    // Step through frames saving only the biggest bottom-up frames
    // thanks to the sort order. This relies on the tree property
    // where children are always smaller than their parents.
    for (var k in keys) {
        var x = parseInt(keys[k]);
        var w = matches[keys[k]];
        if (x >= lastx + lastw) {
            count += w;
            lastx = x;
            lastw = w;
        }
    }
    // display matched percent
    matchedtxt.classList.remove("hide");
    var pct = 100 * count / maxwidth;
    if (pct != 100) pct = pct.toFixed(1);
    matchedtxt.firstChild.nodeValue = "Matched: " + pct + "%";
}
function format_percent(n) {
    return n.toFixed(4) + "%";
}
