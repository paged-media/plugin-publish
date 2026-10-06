// Open $PAGED_OUT_DIR/out.idml, report what InDesign sees, save it as an
// .indd, reopen that and report again, then export the reopened document
// as IDML (out-resaved.idml). Writes $PAGED_OUT_DIR/verify.json.
app.scriptPreferences.userInteractionLevel = UserInteractionLevels.NEVER_INTERACT;
function q(s) {
    var t = String(s).replace(/\\/g, "\\\\").replace(/"/g, '\\"');
    // Control characters (a variable instance is one in `contents`) as
    // \u escapes, so the recording is valid JSON.
    return '"' + t.replace(/[\x00-\x1f]/g, function (c) {
        return "\\u" + ("000" + c.charCodeAt(0).toString(16)).slice(-4);
    }) + '"';
}
function report(doc) {
    var out = [];
    var items = doc.pages[0].allPageItems;
    var its = [];
    for (var i = 0; i < items.length; i++) {
        var it = items[i];
        its.push("{" + ["\"kind\":" + q(it.constructor.name), "\"name\":" + q(it.name),
            "\"visible\":" + it.visible, "\"locked\":" + it.locked].join(",") + "}");
    }
    out.push("\"items\":[" + its.join(",") + "]");
    var vars = [];
    for (var v = 0; v < doc.textVariables.length; v++) {
        var tv = doc.textVariables[v];
        if (tv.variableType != VariableTypes.CUSTOM_TEXT_TYPE) continue;
        vars.push("{" + ["\"name\":" + q(tv.name), "\"contents\":" + q(tv.variableOptions.contents)].join(",") + "}");
    }
    out.push("\"customVariables\":[" + vars.join(",") + "]");
    var inst = [];
    var paths = [];
    for (var s = 0; s < doc.stories.length; s++) {
        var st = doc.stories[s];
        for (var k = 0; k < st.textVariableInstances.length; k++) {
            var ti = st.textVariableInstances[k];
            inst.push("{" + ["\"variable\":" + q(ti.associatedTextVariable.name),
                "\"resultText\":" + q(ti.resultText), "\"story\":" + q(st.contents)].join(",") + "}");
        }
        for (var p = 0; p < st.textContainers.length; p++) {
            var c = st.textContainers[p];
            if (c.constructor.name != "TextPath") continue;
            paths.push("{" + ["\"contents\":" + q(c.contents), "\"overflows\":" + c.overflows,
                "\"startBracket\":" + c.startBracket, "\"endBracket\":" + c.endBracket].join(",") + "}");
        }
    }
    out.push("\"variableInstances\":[" + inst.join(",") + "]");
    out.push("\"textPaths\":[" + paths.join(",") + "]");
    return "{" + out.join(",") + "}";
}
var dir = PAGED_OUT_DIR;
var doc = app.open(File(dir + "/out.idml"), false);
var opened = report(doc);
var indd = File(dir + "/out.indd");
doc.save(indd);
doc.close(SaveOptions.NO);
var doc2 = app.open(indd, false);
var reopened = report(doc2);
doc2.exportFile(ExportFormat.INDESIGN_MARKUP, File(dir + "/out-resaved.idml"));
doc2.close(SaveOptions.NO);
var f = File(dir + "/verify.json");
f.encoding = "UTF-8";
f.open("w");
f.write("{\"indesign\":" + q(app.version) + ",\"opened\":" + opened + ",\"savedAndReopened\":" + reopened + "}");
f.close();
