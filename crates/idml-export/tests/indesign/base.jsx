// The base document for the object-model export probe (see
// ../indesign_object_model.rs): a text frame, a rectangle, a straight
// line. Run by probe.sh; writes $PAGED_OUT_DIR/base.idml.
app.scriptPreferences.userInteractionLevel = UserInteractionLevels.NEVER_INTERACT;
var doc = app.documents.add(false);
doc.viewPreferences.horizontalMeasurementUnits = MeasurementUnits.POINTS;
doc.viewPreferences.verticalMeasurementUnits = MeasurementUnits.POINTS;
var pg = doc.pages[0];
var tf = pg.textFrames.add({geometricBounds: [100, 100, 200, 400]});
tf.contents = "Hello world";
tf.name = "body";
var r = pg.rectangles.add({geometricBounds: [250, 100, 350, 300]});
r.name = "box";
var line = pg.graphicLines.add();
line.paths[0].entirePath = [[100, 450], [400, 450]];
line.name = "baseline";
doc.exportFile(ExportFormat.INDESIGN_MARKUP, File(PAGED_OUT_DIR + "/base.idml"));
doc.close(SaveOptions.NO);
