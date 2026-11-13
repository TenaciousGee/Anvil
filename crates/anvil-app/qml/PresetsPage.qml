// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Anvil contributors (see AUTHORS)
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import com.anvil.app

// Browse presets, see every resolved setting, and edit them: changes are
// saved as your own preset that stores only what differs, plus rules that
// adjust it per source (UHD, HDR, interlaced, long films...).
Item {
    id: page
    property var ctl
    function lines(s) { return s.length ? s.split("\n") : [] }
    function t(plain, forge) { return Prefs.forge ? forge : plain }
    readonly property var allRows: lines(ctl.presets).map(l => l.split("\t"))
    // Presets whose encoder this machine cannot use are hidden (GPU presets
    // without that GPU's encoder, or all GPU presets in CPU-only mode),
    // unless "show all" is on. The preset in use always shows.
    property bool showAll: false
    readonly property var hiddenRows: allRows.filter(r => r[4] && r[0] !== ctl.preset)
    readonly property var rows: showAll ? allRows : allRows.filter(r => !r[4] || r[0] === ctl.preset)
    property string selected: ctl.preset
    readonly property bool selectedIsUser: (rows.find(r => r[0] === selected) || ["", "", "", ""])[3] === "user"

    PresetEditor { id: editor }
    property alias editor: editor

    readonly property var fieldRows: lines(editor.fields).map(l => l.split("\t"))
    readonly property string draftEncoder: (fieldRows.find(r => r[0] === "video.encoder") || ["", "", "", "x265"])[3]
    readonly property var shownFields: fieldRows.filter(r => r[5] === "" || r[5] === draftEncoder)
    readonly property var optRows: lines(editor.options).map(l => l.split("\t"))
    readonly property var optGroups: optRows.map(r => r[2]).filter((g, i, a) => a.indexOf(g) === i)
    readonly property var encoderChoices: [["x265", "x265"], ["x264", "x264"], ["svt_av1", "SVT-AV1"], ["rav1e", "rav1e"], ["nvencc", "NVEncC"], ["qsvencc", "QSVEncC"], ["vceencc", "VCEEncC"], ["ffmpeg_vaapi", "FFmpeg VAAPI"]]
    readonly property var ruleRows: lines(editor.rules).map(l => l.split("\t"))
    readonly property var anyYesNo: [["any", "Any"], ["yes", "Yes"], ["no", "No"]]

    RowLayout {
        anchors.fill: parent
        anchors.margins: 24
        spacing: Tokens.gap

        ColumnLayout {
            Layout.preferredWidth: 340
            Layout.maximumWidth: 340
            Layout.fillWidth: false
            Layout.alignment: Qt.AlignTop
            Layout.fillHeight: true
            spacing: Tokens.gap

            PageHeader {
                Layout.fillWidth: true
                title: Prefs.forge ? "Alloys" : "Presets"
                subtitle: Prefs.forge ? "Each alloy is a recipe for the metal. Edit one to make your own: it is saved in ~/.config/anvil/presets and stores only what it changes."
                                      : "Edit any preset to make your own. It is saved in ~/.config/anvil/presets and stores only what it changes."
            }

            // The list scrolls on its own, with a visible scroll bar whenever
            // there are more presets than fit.
            ScrollView {
                id: presetScroll
                Layout.fillWidth: true
                Layout.fillHeight: true
                contentWidth: availableWidth
                clip: true
                ScrollBar.vertical.policy: contentHeight > height ? ScrollBar.AlwaysOn : ScrollBar.AlwaysOff
                ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

                ColumnLayout {
                    // Room on the right for the scroll bar.
                    width: presetScroll.availableWidth - (presetScroll.contentHeight > presetScroll.height ? 14 : 0)
                    spacing: Tokens.gap
                    Text {
                        Layout.fillWidth: true
                        visible: page.hiddenRows.length > 0
                        text: (page.showAll ? "Showing all presets. " : page.hiddenRows.length + (page.hiddenRows.length === 1 ? " preset is" : " presets are") + " hidden: this machine cannot use their encoder. ")
                              + "<a href='toggle'>" + (page.showAll ? "Hide them" : "Show them") + "</a>"
                        textFormat: Text.StyledText
                        linkColor: Tokens.accent
                        color: Tokens.dim
                        font.pixelSize: 12
                        wrapMode: Text.WordWrap
                        onLinkActivated: page.showAll = !page.showAll
                        HoverHandler { cursorShape: parent.hoveredLink ? Qt.PointingHandCursor : Qt.ArrowCursor }
                    }
                    Repeater {
                        model: page.rows
                        Rectangle {
                            id: presetRow
                            required property var modelData
                            readonly property bool isSelected: modelData[0] === page.selected
                            readonly property bool inUse: modelData[0] === page.ctl.preset
                            Layout.fillWidth: true
                            implicitHeight: col.implicitHeight + 24
                            radius: Tokens.radius
                            color: isSelected ? Tokens.tint : (hover.containsMouse ? Tokens.raised : Tokens.surface)
                            border.color: isSelected ? Tokens.accent : Tokens.border
                            Behavior on color { ColorAnimation { duration: 120 } }

                            ColumnLayout {
                                id: col
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                anchors.margins: 14
                                spacing: 4
                                RowLayout {
                                    Text { text: presetRow.modelData[1]; color: Tokens.text; font.pixelSize: Tokens.heading; font.weight: Font.DemiBold; Layout.fillWidth: true; elide: Text.ElideRight }
                                    Chip { text: presetRow.inUse ? "in use" : (presetRow.modelData[4] ? "not here" : presetRow.modelData[3]); tone: presetRow.inUse ? Tokens.good : Tokens.dim }
                                }
                                Text { text: presetRow.modelData[2]; color: Tokens.dim; font.pixelSize: 12; wrapMode: Text.WordWrap; Layout.fillWidth: true }
                        Text { visible: !!presetRow.modelData[4]; text: presetRow.modelData[4] || ""; color: Tokens.warn; font.pixelSize: 12; wrapMode: Text.WordWrap; Layout.fillWidth: true }
                            }
                            MouseArea {
                                id: hover; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
                                onClicked: { page.selected = presetRow.modelData[0]; if (editor.active && !editor.dirty) editor.discard() }
                            }
                        }
                    }
                }
            }
        }

        // Read-only view
        Card {
            visible: !editor.active
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.alignment: Qt.AlignTop
            title: "All settings"
            trailing: page.selected
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                PrimaryButton {
                    text: page.selected === page.ctl.preset ? "In use" : page.t("Use this preset", "Use this alloy")
                    enabled: page.selected !== page.ctl.preset
                    onClicked: page.ctl.choose(page.selected)
                }
                PrimaryButton { text: page.selectedIsUser ? "Edit" : "Edit a copy"; quiet: true; onClicked: editor.start(page.selected) }
                PrimaryButton { visible: page.selectedIsUser; text: "Delete"; quiet: true; onClicked: if (editor.remove(page.selected)) page.ctl.reloadpresets("") }
                Text {
                    Layout.fillWidth: true
                    text: page.ctl.loaded ? "Applies to the open file and new jobs." : "Applies to the next file you open."
                    color: Tokens.dim
                    font.pixelSize: 12
                    elide: Text.ElideRight
                }
            }
            Text { visible: editor.message.length > 0 && !editor.active; text: editor.message; color: Tokens.warn; font.pixelSize: 12 }
            ScrollView {
                Layout.fillWidth: true
                Layout.preferredHeight: page.height - 200
                clip: true
                TextArea {
                    readOnly: true
                    selectByMouse: true
                    text: page.ctl.describe(page.selected)
                    color: Tokens.text
                    font.family: Tokens.mono
                    font.pixelSize: 12
                    background: Rectangle { color: Tokens.bg; radius: 6; border.color: Tokens.border }
                }
            }
        }

        // Editor
        ScrollView {
            id: editView
            visible: editor.active
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: availableWidth
            clip: true

            ColumnLayout {
                width: editView.availableWidth
                spacing: Tokens.gap

                Card {
                    Layout.fillWidth: true
                    title: "Editing " + editor.base
                    trailing: editor.dirty ? "Unsaved changes" : "No changes yet"
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8
                        Caption { text: "Save as" }
                        Field { id: saveId; Layout.preferredWidth: 220; committedText: editor.suggested }
                        PrimaryButton {
                            text: "Save"
                            onClicked: {
                                const id = editor.save(saveId.text)
                                if (id.length) { page.ctl.reloadpresets(id); page.selected = id }
                            }
                        }
                        PrimaryButton { text: "Close"; quiet: true; onClicked: editor.discard() }
                        Item { Layout.fillWidth: true }
                    }
                    Text { Layout.fillWidth: true; visible: editor.message.length > 0; text: editor.message; color: Tokens.warn; font.pixelSize: 12; wrapMode: Text.WordWrap }
                    Text {
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                        color: Tokens.dim
                        font.pixelSize: 12
                        text: "Saving uses the open file and new jobs straight away. Built-in presets are never changed: edits to one are saved as a new preset of your own."
                    }
                }

                Card {
                    Layout.fillWidth: true
                    title: "Settings"
                    GridLayout {
                        Layout.fillWidth: true
                        columns: 2
                        columnSpacing: 12
                        rowSpacing: 8
                        Repeater {
                            model: page.shownFields
                            delegate: Item {
                                id: cell
                                required property var modelData
                                readonly property string kind: modelData[2]
                                Layout.fillWidth: true
                                Layout.columnSpan: (modelData[0] === "description") ? 2 : 1
                                implicitHeight: box.implicitHeight
                                ColumnLayout {
                                    id: box
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    spacing: 4
                                    Caption { text: cell.modelData[1]; visible: cell.kind !== "bool" }
                                    Field {
                                        visible: cell.kind === "text" || cell.kind === "number" || cell.kind === "list"
                                        Layout.fillWidth: true
                                        committedText: cell.modelData[3]
                                        onCommitted: (v) => editor.set(cell.modelData[0], v)
                                    }
                                    Select {
                                        visible: cell.kind === "choice"
                                        Layout.fillWidth: true
                                        options: cell.modelData[4].length ? cell.modelData[4].split("|").map(o => o.split("=")) : []
                                        value: cell.modelData[3]
                                        onPicked: (v) => editor.set(cell.modelData[0], v)
                                    }
                                    Toggle {
                                        visible: cell.kind === "bool"
                                        text: cell.modelData[1]
                                        checked: cell.modelData[3] === "true"
                                        onToggled: editor.set(cell.modelData[0], checked ? "true" : "false")
                                    }
                                }
                            }
                        }
                    }
                }

                Card {
                    Layout.fillWidth: true
                    title: "Encoder options"
                    trailing: page.optRows.filter(r => r[4] !== "").length + " changed from the encoder's defaults"
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 8
                        Caption { text: "Options for" }
                        Select {
                            Layout.preferredWidth: 200
                            options: page.encoderChoices
                            value: editor.optencoder
                            onPicked: (v) => editor.optview(v)
                        }
                        Text {
                            Layout.fillWidth: true
                            text: editor.optencoder === page.draftEncoder ? "The encoder this preset uses." : "Not the encoder this preset uses: these apply if you switch to it."
                            color: Tokens.dim; font.pixelSize: 12; wrapMode: Text.WordWrap
                        }
                    }
                    Repeater {
                        model: page.optGroups
                        ColumnLayout {
                            id: grp
                            required property var modelData
                            Layout.fillWidth: true
                            spacing: 6
                            Text { text: grp.modelData; color: Tokens.text; font.pixelSize: Tokens.body; font.weight: Font.DemiBold; Layout.topMargin: 6 }
                            GridLayout {
                                Layout.fillWidth: true
                                columns: 2
                                columnSpacing: 16
                                rowSpacing: 6
                                Repeater {
                                    model: page.optRows.filter(r => r[2] === grp.modelData)
                                    delegate: RowLayout {
                                        id: opt
                                        required property var modelData
                                        readonly property string kind: modelData[3]
                                        readonly property bool changed: modelData[4] !== ""
                                        Layout.fillWidth: true
                                        Layout.preferredWidth: 1
                                        spacing: 8
                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: 0
                                            Text { text: opt.modelData[1] + (opt.changed ? "  •" : ""); color: opt.changed ? Tokens.accent : Tokens.text; font.pixelSize: 13; elide: Text.ElideRight; Layout.fillWidth: true }
                                            Text { text: opt.modelData[0] + " · default " + opt.modelData[5] + " · " + opt.modelData[9]; color: Tokens.dim; font.pixelSize: 11; elide: Text.ElideRight; Layout.fillWidth: true
                                                   ToolTip.visible: tipArea.containsMouse; ToolTip.text: opt.modelData[9]
                                                   MouseArea { id: tipArea; anchors.fill: parent; hoverEnabled: true } }
                                        }
                                        Field {
                                            visible: opt.kind === "int" || opt.kind === "float" || opt.kind === "text"
                                            Layout.preferredWidth: 90
                                            committedText: opt.modelData[4]
                                            placeholderText: opt.modelData[5]
                                            onCommitted: (v) => editor.optset(opt.modelData[0], v)
                                        }
                                        Select {
                                            visible: opt.kind === "choice" || opt.kind === "toggle" || opt.kind === "flag"
                                            Layout.preferredWidth: 130
                                            options: [["", "Default (" + opt.modelData[5] + ")"]].concat(
                                                opt.kind === "choice" ? opt.modelData[6].split("|").map(c => [c, c]) : [["on", "On"], ["off", "Off"]])
                                            value: opt.modelData[4]
                                            onPicked: (v) => editor.optset(opt.modelData[0], v)
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Caption { text: "Other arguments (passed as typed)" }
                    Field {
                        Layout.fillWidth: true
                        font.family: Tokens.mono
                        committedText: editor.other
                        placeholderText: "none"
                        onCommitted: (v) => editor.optother(v)
                    }
                }

                Card {
                    Layout.fillWidth: true
                    title: "Rules"
                    trailing: page.ruleRows.length === 0 ? "None" : page.ruleRows.length + (page.ruleRows.length === 1 ? " rule" : " rules")
                    Text {
                        Layout.fillWidth: true
                        wrapMode: Text.WordWrap
                        color: Tokens.dim
                        font.pixelSize: 12
                        text: "A rule changes settings only for sources that match every condition, applied in order before your per-job changes. Settings use the names shown under All settings, for example video.x265.preset = slow; video.x265.crf.uhd = 19. Filters: denoise, deband, sharpen, grain, qtgmc."
                    }
                    Repeater {
                        id: ruleRep
                        model: page.ruleRows
                        Rectangle {
                            id: rule
                            required property var modelData
                            required property int index
                            Layout.fillWidth: true
                            implicitHeight: rcol.implicitHeight + 20
                            radius: 6
                            color: Tokens.raised
                            ColumnLayout {
                                id: rcol
                                anchors.fill: parent
                                anchors.margins: 10
                                spacing: 8
                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: 8
                                    Text { text: (rule.index + 1) + "."; color: Tokens.dim; font.pixelSize: Tokens.body }
                                    Field { Layout.fillWidth: true; committedText: rule.modelData[0]; onCommitted: (v) => editor.ruleset(rule.index, "name", v) }
                                    Text { text: "when " + rule.modelData[10]; color: Tokens.dim; font.pixelSize: 12 }
                                    IconButton { text: "▲"; enabled: rule.index > 0; onClicked: editor.rulemove(rule.index, rule.index - 1) }
                                    IconButton { text: "▼"; enabled: rule.index < ruleRep.count - 1; onClicked: editor.rulemove(rule.index, rule.index + 1) }
                                    IconButton { text: "✕"; onClicked: editor.ruleremove(rule.index) }
                                }
                                GridLayout {
                                    Layout.fillWidth: true
                                    columns: 7
                                    columnSpacing: 8
                                    rowSpacing: 2
                                    Caption { text: "Resolution" }
                                    Caption { text: "HDR" }
                                    Caption { text: "Dolby Vision" }
                                    Caption { text: "Interlaced" }
                                    Caption { text: "Longer than (min)" }
                                    Caption { text: "Shorter than (min)" }
                                    Caption { text: "Larger than (GB)" }
                                    Select { Layout.fillWidth: true; options: [["any", "Any"], ["sd", "SD"], ["hd", "HD"], ["uhd", "UHD"]]; value: rule.modelData[1]; onPicked: (v) => editor.ruleset(rule.index, "resolution", v) }
                                    Select { Layout.fillWidth: true; options: page.anyYesNo; value: rule.modelData[2]; onPicked: (v) => editor.ruleset(rule.index, "hdr", v) }
                                    Select { Layout.fillWidth: true; options: page.anyYesNo; value: rule.modelData[3]; onPicked: (v) => editor.ruleset(rule.index, "dv", v) }
                                    Select { Layout.fillWidth: true; options: page.anyYesNo; value: rule.modelData[4]; onPicked: (v) => editor.ruleset(rule.index, "interlaced", v) }
                                    Field { Layout.fillWidth: true; committedText: rule.modelData[5]; onCommitted: (v) => editor.ruleset(rule.index, "longer", v) }
                                    Field { Layout.fillWidth: true; committedText: rule.modelData[6]; onCommitted: (v) => editor.ruleset(rule.index, "shorter", v) }
                                    Field { Layout.fillWidth: true; committedText: rule.modelData[7]; onCommitted: (v) => editor.ruleset(rule.index, "larger", v) }
                                }
                                Caption { text: "Then set" }
                                Field { Layout.fillWidth: true; font.family: Tokens.mono; committedText: rule.modelData[8]; onCommitted: (v) => editor.ruleset(rule.index, "settings", v) }
                                Caption { text: "And add filters" }
                                Field { Layout.fillWidth: true; committedText: rule.modelData[9]; placeholderText: "none"; onCommitted: (v) => editor.ruleset(rule.index, "filters", v) }
                            }
                        }
                    }
                    PrimaryButton { text: "Add rule"; quiet: true; onClicked: editor.ruleadd() }
                }

                Card {
                    Layout.fillWidth: true
                    title: "What will be saved"
                    trailing: "~/.config/anvil/presets/" + (saveId.text.length ? saveId.text : "…") + ".toml"
                    TextArea {
                        Layout.fillWidth: true
                        readOnly: true
                        selectByMouse: true
                        text: editor.preview
                        color: Tokens.text
                        font.family: Tokens.mono
                        font.pixelSize: 12
                        wrapMode: TextEdit.NoWrap
                        background: Rectangle { color: Tokens.bg; radius: 6; border.color: Tokens.border }
                    }
                }
            }
        }
    }
}
