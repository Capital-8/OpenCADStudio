# OpenCADStudio - Unreleased (2026-10-25 to Present)

> **Summary:** Added dimension association controls and converter-based file interchange with explicit review of content a conversion cannot preserve.

### 🌟 Notable Release Notes

* **Dimension and Constraint Controls:** Added `DIMOVERRIDE`, `DIMDISASSOCIATE`, `DIMREASSOCIATE`, and `DCDISPLAY`, plus `QPMODE` for Quick Properties. Reassociation anchors dimensions to geometry at their definition points and reports partial results. *(Contributed by [@HakanSeven12](https://github.com/HakanSeven12))*
* **Additional File Formats:** Added open and save workflows through OpenCADDocument converters. Before a save that would lose converter-reported drawing content, the app lists the losses and lets you return or continue. *(Contributed by [@HakanSeven12](https://github.com/HakanSeven12))*
* **Recovery Diagnostics:** Recovery reports now identify the acadrust reader revision used to read the drawing. *(Contributed by [@HakanSeven12](https://github.com/HakanSeven12))*

### 🚀 New Features

* **Dimension Overrides and Associations:** Set or clear per-dimension variable overrides; remove a dimension's association or reassociate it to geometry. `DCDISPLAY` toggles constraint indicators on selected objects, and `QPMODE` reads or sets Quick Properties visibility.
* **Converter Interchange and Loss Review:** Open and save supported external formats through registered converters. Review converter-reported losses before choosing to proceed with a save.

### 🐛 Bug Fixes

* **Partial Dimension Reassociation Reporting:** Identify dimensions that could only be partly re-anchored.
* **Dimension and Preview Precision at Large Coordinates:** Dimensions are now built in a local frame and folded back into their double-single vertex buffers, so their lines, arrows and extension lines stay on the points they measure on UTM-scale drawings. Measurement and drafting previews likewise keep full precision instead of downcasting absolute coordinates to f32, which had left the `DIST` rubber band offset from the picked points.
* **Recovery Reader Revision:** Read the embedded acadrust source hash so recovery reports show the reader revision following the shared document-model migration.
