import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ApplicationWindow {
    id: window
    width: 1150
    height: 740
    minimumWidth: 760
    minimumHeight: 520
    visible: true
    title: "untethered"
    color: "#1d1c21"
    palette.window: window.color
    palette.base: window.fieldSurface
    palette.text: "#f0edf3"
    palette.windowText: palette.text
    palette.button: window.hoverSurface
    palette.buttonText: palette.text
    palette.placeholderText: window.muted
    palette.highlight: "#34524a"
    palette.highlightedText: palette.text
    readonly property color muted: "#aaa5b2"
    readonly property color accent: "#9be1c9"
    readonly property color activeSurface: "#37343f"
    readonly property color hoverSurface: "#2c2933"
    readonly property color sidebarSurface: "#17161b"
    readonly property color railSurface: "#111015"
    readonly property color fieldSurface: "#25232b"
    readonly property color divider: "#34313b"
    readonly property string directContext: "direct"
    readonly property string contextName: contextId === directContext ? "Direct messages"
        : contextId === "" ? "All rooms" : (spaces.find(space => space.id === contextId)?.name ?? "Space")
    readonly property var roomIndex: chat.roomList
    readonly property var spaces: roomIndex.filter(room => room.space)
    readonly property var visibleRooms: roomsFor(contextId)
    property string contextId: ""
    property var lastRooms: ({})
    property var drafts: ({})
    property string draftRoom: ""
    property bool hadSession: false
    property var replyDrafts: ({})
    property var replyTarget: null

    function startReply(message) {
        replyTarget = message
        composer.forceActiveFocus()
    }

    function roomsFor(context) {
        return roomIndex.filter(room => !room.space && (context === ""
            || (context === directContext ? room.dm : chat.inSpace(context, room.id))))
    }

    function navigate(context, roomId) {
        if (chat.selected !== "")
            lastRooms[contextId] = chat.selected

        const candidates = roomsFor(context)
        const target = candidates.find(room => room.id === roomId)
            ?? candidates.find(room => room.id === lastRooms[context])
            ?? candidates[0]
        contextId = context
        chat.openRoom(target?.id ?? "")
    }

    function openSearchResult(room) {
        if (room.space) {
            navigate(room.id)
        } else if (room.dm) {
            navigate(directContext, room.id)
        } else {
            const space = spaces.find(candidate => candidate.id === contextId
                && chat.inSpace(candidate.id, room.id))
                ?? spaces.find(candidate => chat.inSpace(candidate.id, room.id))
            navigate(space?.id ?? "", room.id)
        }
    }

    Connections {
        target: chat
        function onChanged() {
            if (!chat.loggedIn) {
                if (!window.hadSession)
                    return
                window.hadSession = false
                imageViewer.close()
                logoutConfirmation.close()
                settings.close()
                switcher.close()
                window.contextId = ""
                window.lastRooms = ({})
                window.drafts = ({})
                window.draftRoom = ""
                window.replyDrafts = ({})
                window.replyTarget = null
                composer.clear()
                recovery.clear()
                password.clear()
                return
            }
            window.hadSession = true
            if (window.draftRoom !== chat.selected) {
                imageViewer.close()
                if (window.draftRoom !== "") {
                    window.drafts[window.draftRoom] = composer.text
                    window.replyDrafts[window.draftRoom] = window.replyTarget
                }
                window.draftRoom = chat.selected
                composer.text = window.drafts[chat.selected] ?? ""
                window.replyTarget = window.replyDrafts[chat.selected] ?? null
            }
            Qt.callLater(window.reconcileNavigation)
        }
    }

    function reconcileNavigation() {
        if (!chat.loggedIn)
            return
        if (contextId !== "" && contextId !== directContext
                && !spaces.some(space => space.id === contextId)) {
            navigate("")
        } else if (!visibleRooms.some(room => room.id === chat.selected)) {
            navigate(contextId)
        }
    }

    component Avatar: Rectangle {
        id: avatar
        property url source
        property string name: ""
        property string fallbackText: Array.from(name.trim()).slice(0, 2).join("").toUpperCase()
        radius: width / 3
        color: window.activeSurface

        Label {
            anchors.centerIn: parent
            visible: picture.status !== Image.Ready
            text: avatar.fallbackText
            textFormat: Text.PlainText
            color: window.accent
            font.pixelSize: Math.max(10, avatar.width * 0.36)
            font.bold: true
        }
        Image {
            id: picture
            anchors.fill: parent
            anchors.margins: 2
            source: avatar.source
            asynchronous: true
            visible: status === Image.Ready
            fillMode: Image.PreserveAspectFit
        }
    }

    component NavigationButton: ItemDelegate {
        id: navigationButton
        property bool centered: false
        font.pixelSize: 14
        property url avatarSource
        property string avatarName: ""
        property bool showAvatar: false
        contentItem: RowLayout {
            spacing: 8
            Avatar {
                visible: navigationButton.showAvatar
                source: navigationButton.avatarSource
                name: navigationButton.avatarName
                Layout.preferredWidth: 24
                Layout.preferredHeight: 24
            }
            Label {
                Layout.fillWidth: true
                text: navigationButton.text
                textFormat: Text.PlainText
                color: navigationButton.centered && navigationButton.highlighted ? window.accent
                    : navigationButton.highlighted || navigationButton.font.bold || navigationButton.hovered
                        ? window.palette.text : window.muted
                font: navigationButton.font
                elide: Text.ElideRight
                horizontalAlignment: navigationButton.centered ? Text.AlignHCenter : Text.AlignLeft
                verticalAlignment: Text.AlignVCenter
            }
        }
        background: Rectangle {
            radius: 8
            color: navigationButton.highlighted ? window.activeSurface
                : navigationButton.hovered || navigationButton.visualFocus ? window.hoverSurface : "transparent"
            border.width: navigationButton.visualFocus ? 1 : 0
            border.color: window.accent
        }
    }

    Shortcut {
        sequence: "Ctrl+K"
        enabled: chat.loggedIn && !settings.opened && !imageViewer.opened
        onActivated: switcher.open()
    }

    Shortcut {
        sequence: "Ctrl+,"
        enabled: chat.loggedIn && !switcher.opened && !settings.opened && !imageViewer.opened
        onActivated: settings.open()
    }

    Dialog {
        id: settings
        parent: Overlay.overlay
        anchors.centerIn: parent
        width: Math.min(window.width - 48, 500)
        padding: 20
        title: "User settings"
        modal: true
        standardButtons: Dialog.Close
        onClosed: recovery.clear()

        contentItem: ColumnLayout {
            spacing: 14
            RowLayout {
                Layout.fillWidth: true
                spacing: 12
                Avatar {
                    source: chat.profileAvatar
                    name: chat.profileName.startsWith("@") ? chat.profileName.slice(1) : chat.profileName
                    Layout.preferredWidth: 48
                    Layout.preferredHeight: 48
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 4
                    Label {
                        text: chat.profileName
                        textFormat: Text.PlainText
                        font.pixelSize: 18
                        font.weight: Font.DemiBold
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                    }
                    TextEdit {
                        text: chat.profile
                        textFormat: TextEdit.PlainText
                        readOnly: true
                        selectByMouse: true
                        wrapMode: TextEdit.Wrap
                        color: window.muted
                        selectionColor: window.palette.highlight
                        selectedTextColor: window.palette.highlightedText
                        Layout.fillWidth: true
                        Accessible.name: "Matrix account ID"
                    }
                }
            }
            Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
            Label {
                text: "Device verification"
                font.weight: Font.DemiBold
            }
            Label {
                text: chat.verified ? "This device is verified" : "This device is not verified"
                color: chat.verified ? window.accent : window.muted
            }
            Label {
                visible: chat.deviceId !== ""
                text: "Device ID: " + chat.deviceId
                textFormat: Text.PlainText
                wrapMode: Text.Wrap
                color: window.muted
                Layout.fillWidth: true
            }
            Label {
                visible: !chat.verified
                text: "Use your Matrix recovery key to verify this device."
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }
            RowLayout {
                visible: !chat.verified
                Layout.fillWidth: true
                TextField {
                    id: recovery
                    placeholderText: "Matrix recovery key"
                    Accessible.name: placeholderText
                    echoMode: TextInput.Password
                    enabled: chat.ready
                    Layout.fillWidth: true
                    onAccepted: if (verifyDevice.enabled) verifyDevice.clicked()
                }
                Button {
                    id: verifyDevice
                    text: "Verify device"
                    enabled: chat.ready && recovery.text.trim().length > 0
                    onClicked: {
                        chat.verify(recovery.text)
                        recovery.clear()
                    }
                }
            }
            Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
            Button {
                text: chat.loggingOut ? "Signing out…" : "Sign out"
                enabled: chat.ready && !chat.loggingOut
                onClicked: logoutConfirmation.open()
            }
            Label {
                visible: text.length > 0
                text: chat.status
                textFormat: Text.PlainText
                wrapMode: Text.WordWrap
                color: window.muted
                Layout.fillWidth: true
            }
        }
    }

    Dialog {
        id: imageViewer
        parent: Overlay.overlay
        anchors.centerIn: parent
        title: "Image"
        width: Math.min(window.width - 48, 1050)
        height: Math.min(window.height - 48, 850)
        modal: true
        standardButtons: Dialog.Close
        property url imageSource
        onClosed: imageSource = ""

        contentItem: Item {
            Image {
                id: enlargedImage
                anchors.fill: parent
                source: imageViewer.imageSource
                asynchronous: true
                fillMode: Image.PreserveAspectFit
                sourceSize.width: 1920
                sourceSize.height: 1920
            }
            Label {
                anchors.centerIn: parent
                visible: enlargedImage.status !== Image.Ready
                color: window.muted
                text: enlargedImage.status === Image.Loading ? "Loading image…" : "Image unavailable"
            }
        }
    }

    Dialog {
        id: logoutConfirmation
        parent: Overlay.overlay
        anchors.centerIn: parent
        width: Math.min(window.width - 48, 440)
        padding: 20
        title: "Sign out of Matrix?"
        modal: true
        onAccepted: chat.logout()

        contentItem: Label {
            text: "Signing out removes this device’s local encryption keys and cached messages. Keep your Matrix recovery key so you can recover backed-up keys after signing in again."
            wrapMode: Text.WordWrap
        }

        footer: DialogButtonBox {
            Button {
                text: "Cancel"
                DialogButtonBox.buttonRole: DialogButtonBox.RejectRole
            }
            Button {
                text: "Sign out"
                enabled: chat.ready && !chat.loggingOut
                DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
            }
        }
    }

    Dialog {
        id: switcher
        parent: Overlay.overlay
        anchors.centerIn: parent
        width: Math.min(window.width - 48, 520)
        modal: true
        title: "Jump to a room or space"
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside

        function acceptResult(room) {
            if (room) {
                close()
                window.openSearchResult(room)
            }
        }

        onOpened: {
            query.clear()
            results.currentIndex = 0
            query.forceActiveFocus()
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            TextField {
                id: query
                Layout.fillWidth: true
                placeholderText: "Search spaces, channels and DMs"
                Accessible.name: placeholderText
                onTextChanged: results.currentIndex = 0
                Keys.onUpPressed: results.currentIndex = Math.max(0, results.currentIndex - 1)
                Keys.onDownPressed: results.currentIndex = Math.min(results.count - 1, results.currentIndex + 1)
                onAccepted: switcher.acceptResult(results.model[results.currentIndex])
            }
            ListView {
                id: results
                Layout.fillWidth: true
                Layout.preferredHeight: 280
                clip: true
                model: window.roomIndex.filter(room => room.name.toLowerCase().includes(query.text.trim().toLowerCase()))
                delegate: NavigationButton {
                    required property var modelData
                    required property int index
                    width: ListView.view.width
                    text: (modelData.space ? "Space  ·  " : modelData.dm ? "DM  ·  " : "#  ") + modelData.name
                    highlighted: results.currentIndex === index
                    onClicked: switcher.acceptResult(modelData)
                }
                ScrollBar.vertical: ScrollBar {}
                Label {
                    anchors.centerIn: parent
                    visible: results.count === 0
                    color: window.muted
                    text: "No matching rooms or spaces"
                }
            }
        }
    }

    ColumnLayout {
        anchors.centerIn: parent
        width: 350
        spacing: 12
        visible: !chat.loggedIn

        Label { text: window.title; font.pixelSize: 34; font.weight: Font.DemiBold }
        Label { text: chat.status; wrapMode: Text.WordWrap; Layout.fillWidth: true; color: window.muted }
        TextField { id: server; placeholderText: "https://matrix.example.org"; Layout.fillWidth: true; enabled: chat.ready }
        TextField { id: user; placeholderText: "Matrix username"; Layout.fillWidth: true; enabled: chat.ready }
        TextField {
            id: password
            placeholderText: "Matrix password"
            echoMode: TextInput.Password
            Layout.fillWidth: true
            enabled: chat.ready
            onAccepted: signIn.clicked()
        }
        Button {
            id: signIn
            text: "Sign in"
            enabled: chat.ready && password.text.length > 0
            onClicked: {
                chat.login(server.text, user.text, password.text)
                password.clear()
            }
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0
        visible: chat.loggedIn

        Rectangle {
            Layout.preferredWidth: 68
            Layout.fillHeight: true
            color: window.railSurface
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 10
                spacing: 6
                NavigationButton {
                    text: "DM"
                    centered: true
                    font.bold: true
                    Layout.fillWidth: true
                    highlighted: window.contextId === window.directContext
                    Accessible.name: "Direct messages"
                    ToolTip.text: Accessible.name
                    ToolTip.visible: hovered
                    onClicked: window.navigate(window.directContext)
                }
                NavigationButton {
                    text: "All"
                    centered: true
                    font.bold: true
                    Layout.fillWidth: true
                    highlighted: window.contextId === ""
                    Accessible.name: "All rooms"
                    ToolTip.text: Accessible.name
                    ToolTip.visible: hovered
                    onClicked: window.navigate("")
                }
                Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
                ListView {
                    id: spaceList
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    spacing: 6
                    model: window.spaces
                    delegate: NavigationButton {
                        required property var modelData
                        width: spaceList.width
                        height: 42
                        centered: true
                        font.bold: true
                        padding: 3
                        contentItem: Avatar {
                            name: modelData.name
                            source: modelData.avatar ?? ""
                        }
                        highlighted: window.contextId === modelData.id
                        Accessible.name: modelData.name
                        ToolTip.text: modelData.name
                        ToolTip.visible: hovered
                        onClicked: window.navigate(modelData.id)
                    }
                }
            }
        }
        Rectangle {
            Layout.preferredWidth: 232
            Layout.fillHeight: true
            color: window.sidebarSurface
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 10
                spacing: 6
                Label {
                    text: window.contextName
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                    Layout.leftMargin: 8
                    Layout.preferredHeight: 38
                    verticalAlignment: Text.AlignVCenter
                    font.pixelSize: 16
                    font.weight: Font.DemiBold
                }
                NavigationButton {
                    text: "Quick switch  ·  Ctrl+K"
                    Layout.fillWidth: true
                    onClicked: switcher.open()
                }
                ListView {
                    id: rooms
                    Layout.fillHeight: true
                    Layout.fillWidth: true
                    clip: true
                    spacing: 2
                    model: window.visibleRooms
                    delegate: NavigationButton {
                        required property var modelData
                        width: rooms.width
                        highlighted: chat.selected === modelData.id
                        avatarSource: modelData.avatar ?? ""
                        avatarName: modelData.name
                        showAvatar: modelData.dm || avatarSource.toString() !== ""
                        text: (modelData.unread ? "•  " : "") + (showAvatar ? "" : "#  ") + modelData.name
                        font.bold: modelData.unread
                        Accessible.name: modelData.name + (modelData.unread ? ", unread" : "")
                        onClicked: window.navigate(window.contextId, modelData.id)
                    }
                    ScrollBar.vertical: ScrollBar {}
                    Label {
                        anchors.centerIn: parent
                        visible: rooms.count === 0
                        color: window.muted
                        text: window.contextId === window.directContext ? "No direct messages" : "No rooms"
                    }
                }
                Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
                RowLayout {
                    Layout.fillWidth: true
                    Layout.margins: 8
                    spacing: 8
                    Avatar {
                        source: chat.profileAvatar
                        name: chat.profileName.startsWith("@") ? chat.profileName.slice(1) : chat.profileName
                        Layout.preferredWidth: 30
                        Layout.preferredHeight: 30
                    }
                    Label {
                        text: chat.profileName
                        textFormat: Text.PlainText
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                        color: window.muted
                        ToolTip.text: chat.profile
                        ToolTip.visible: profileHover.hovered
                        HoverHandler { id: profileHover }
                    }
                    ToolButton {
                        text: "⚙"
                        Accessible.name: "User settings"
                        ToolTip.text: "User settings · Ctrl+,"
                        ToolTip.visible: hovered
                        onClicked: settings.open()
                    }
                }
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 52
                color: window.color
                Rectangle {
                    anchors.bottom: parent.bottom
                    width: parent.width
                    height: 1
                    color: window.divider
                }
                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 20
                    anchors.rightMargin: 18
                    Label {
                        text: chat.roomName || "Choose a room"
                        textFormat: Text.PlainText
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                        font.pixelSize: 17
                        font.weight: Font.DemiBold
                    }
                    Label { text: "Encrypted"; color: window.accent; visible: chat.encrypted }
                    Button { text: "Older messages"; visible: chat.selected !== ""; enabled: chat.ready; onClicked: chat.paginate(true) }
                }
            }
            Button {
                visible: !chat.verified
                text: "Verify this device in User settings"
                Layout.leftMargin: 20
                Layout.topMargin: 8
                onClicked: settings.open()
            }
            Label {
                visible: text.length > 0
                text: chat.status
                color: window.muted
                Layout.fillWidth: true
                Layout.leftMargin: 20
                wrapMode: Text.WordWrap
            }
            ListView {
                id: timeline
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 4
                property string displayedRoom: ""
                property string historyAnchor: ""
                property var eventIndices: ({})
                property string highlightedEvent: ""

                function jumpToEvent(eventId) {
                    const index = eventIndices[eventId]
                    if (index === undefined)
                        return
                    positionViewAtIndex(index, ListView.Center)
                    highlightedEvent = eventId
                    replyHighlight.restart()
                }

                Timer {
                    id: replyHighlight
                    interval: 1600
                    onTriggered: timeline.highlightedEvent = ""
                }

                function refreshMessages() {
                    const messages = chat.messages
                    const sameRoom = displayedRoom === chat.selected
                    const anchor = historyAnchor
                    historyAnchor = ""
                    if (sameRoom && JSON.stringify(model) === JSON.stringify(messages))
                        return

                    if (!sameRoom)
                        highlightedEvent = ""
                    displayedRoom = chat.selected
                    const indices = Object.create(null)
                    messages.forEach((message, index) => {
                        if (message.eventId)
                            indices[message.eventId] = index
                    })
                    eventIndices = indices
                    model = messages
                    forceLayout()
                    const anchorIndex = sameRoom && anchor !== ""
                        ? messages.findIndex(message => message.id === anchor) : -1
                    if (anchorIndex >= 0)
                        positionViewAtIndex(anchorIndex, ListView.Beginning)
                    else
                        positionViewAtEnd()
                }

                Connections {
                    target: chat
                    function onOlderMessagesLoading() {
                        const top = timeline.contentY
                        let index = timeline.indexAt(1, top)
                        if (index < 0)
                            index = timeline.indexAt(1, top + timeline.spacing + 1)
                        timeline.historyAnchor = timeline.model?.[index]?.id ?? ""
                    }
                    function onTimelineChanged() { timeline.refreshMessages() }
                }
                delegate: Item {
                    id: message
                    required property var modelData
                    width: timeline.width
                    height: messageColumn.implicitHeight + (modelData.startsGroup ? 10 : 0)
                    readonly property var original: modelData.replyTo
                        ? timeline.model?.[timeline.eventIndices[modelData.replyTo]] : null

                    HoverHandler { id: messageHover }
                    Rectangle {
                        anchors.fill: parent
                        color: window.activeSurface
                        visible: timeline.highlightedEvent !== "" && timeline.highlightedEvent === modelData.eventId
                    }

                    ColumnLayout {
                        id: messageColumn
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.top: parent.top
                        anchors.leftMargin: 18
                        anchors.rightMargin: 22
                        anchors.topMargin: modelData.startsGroup ? 10 : 0
                        spacing: 6

                        RowLayout {
                            visible: modelData.day.length > 0
                            Layout.fillWidth: true
                            Layout.bottomMargin: 6
                            spacing: 10
                            Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
                            Label { text: modelData.day; color: window.muted; font.pixelSize: 12 }
                            Rectangle { Layout.fillWidth: true; height: 1; color: window.divider }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 10

                            Item {
                                Layout.preferredWidth: 34
                                Layout.preferredHeight: modelData.startsGroup ? 34 : 0
                                Layout.alignment: Qt.AlignTop

                                Avatar {
                                    anchors.fill: parent
                                    visible: modelData.startsGroup
                                    source: visible ? modelData.avatar ?? "" : ""
                                    name: modelData.name
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 3
                                RowLayout {
                                    visible: modelData.startsGroup
                                    Layout.fillWidth: true
                                    spacing: 8
                                    Label {
                                        text: modelData.name
                                        textFormat: Text.PlainText
                                        font.weight: Font.DemiBold
                                        elide: Text.ElideRight
                                        Layout.maximumWidth: message.width / 2
                                        ToolTip.text: modelData.sender
                                        ToolTip.visible: senderHover.hovered
                                        HoverHandler { id: senderHover }
                                    }
                                    Label { text: modelData.time; color: window.muted; font.pixelSize: 12 }
                                    Item { Layout.fillWidth: true }
                                }
                                NavigationButton {
                                    visible: !!modelData.replyTo
                                    enabled: !!message.original
                                    Layout.fillWidth: true
                                    text: message.original
                                        ? message.original.name + " · " + message.original.body.replace(/\s+/g, " ")
                                        : "Original message not loaded"
                                    Accessible.name: "In reply to " + text
                                    onClicked: timeline.jumpToEvent(modelData.replyTo)
                                }
                                Loader {
                                    active: !!modelData.hasImage
                                    visible: active
                                    Layout.fillWidth: true
                                    Layout.maximumWidth: 360
                                    Layout.preferredHeight: 230
                                    Layout.alignment: Qt.AlignLeft

                                    sourceComponent: Button {
                                        id: imagePreview
                                        readonly property url fullSource: modelData.image ?? ""
                                        readonly property url thumbnailSource: modelData.thumbnail ?? ""
                                        property bool thumbnailFailed: false
                                        onThumbnailSourceChanged: thumbnailFailed = false
                                        enabled: fullSource.toString() !== ""
                                        Accessible.name: "Open image"
                                        onClicked: {
                                            imageViewer.imageSource = fullSource
                                            imageViewer.open()
                                        }
                                        background: Rectangle {
                                            radius: 8
                                            color: window.fieldSurface
                                            border.color: imagePreview.visualFocus ? window.accent : window.divider
                                        }
                                        contentItem: Item {
                                            Image {
                                                id: thumbnailImage
                                                anchors.fill: parent
                                                source: !imagePreview.thumbnailFailed && imagePreview.thumbnailSource.toString() !== ""
                                                    ? imagePreview.thumbnailSource : imagePreview.fullSource
                                                asynchronous: true
                                                fillMode: Image.PreserveAspectFit
                                                sourceSize.width: 720
                                                sourceSize.height: 460
                                                onStatusChanged: {
                                                    if (status === Image.Error && source.toString() === imagePreview.thumbnailSource.toString())
                                                        imagePreview.thumbnailFailed = true
                                                }
                                            }
                                            Label {
                                                anchors.centerIn: parent
                                                visible: thumbnailImage.status !== Image.Ready
                                                color: window.muted
                                                text: thumbnailImage.status === Image.Loading ? "Loading image…" : "Image unavailable"
                                            }
                                        }
                                    }
                                }
                                RowLayout {
                                    Layout.fillWidth: true
                                    Label {
                                        text: modelData.body
                                        textFormat: Text.PlainText
                                        wrapMode: Text.Wrap
                                        Layout.fillWidth: true
                                        Layout.alignment: Qt.AlignTop
                                        font.pixelSize: 15
                                        Accessible.name: modelData.name + ", " + modelData.time + ": " + text
                                    }
                                    Rectangle {
                                        id: messageActions
                                        Layout.preferredWidth: actionButtons.implicitWidth + 8
                                        Layout.preferredHeight: actionButtons.implicitHeight + 4
                                        Layout.alignment: Qt.AlignTop
                                        radius: 7
                                        color: window.fieldSurface
                                        border.color: window.divider
                                        opacity: messageHover.hovered || replyAction.activeFocus
                                            || copyAction.activeFocus ? 1 : 0

                                        RowLayout {
                                            id: actionButtons
                                            anchors.centerIn: parent
                                            spacing: 2

                                            ToolButton {
                                                id: replyAction
                                                visible: !!modelData.canReply
                                                enabled: chat.ready && !chat.loggingOut
                                                text: "↩"
                                                Accessible.name: "Reply to " + modelData.name
                                                ToolTip.text: "Reply"
                                                ToolTip.visible: messageActions.opacity > 0 && hovered
                                                Layout.preferredWidth: 30
                                                Layout.preferredHeight: 24
                                                onClicked: window.startReply(modelData)
                                            }
                                            ToolButton {
                                                id: copyAction
                                                text: "Copy"
                                                Accessible.name: "Copy message text"
                                                Layout.preferredHeight: 24
                                                onClicked: chat.copyText(modelData.body)
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                ScrollBar.vertical: ScrollBar {}
            }
            RowLayout {
                visible: window.replyTarget !== null
                Layout.fillWidth: true
                Layout.leftMargin: 16
                Layout.rightMargin: 16
                Layout.topMargin: 8
                Label {
                    text: window.replyTarget
                        ? "Replying to " + window.replyTarget.name + " · " + window.replyTarget.body.replace(/\s+/g, " ")
                        : ""
                    textFormat: Text.PlainText
                    elide: Text.ElideRight
                    color: window.muted
                    Layout.fillWidth: true
                }
                ToolButton {
                    text: "×"
                    Accessible.name: "Cancel reply"
                    onClicked: window.replyTarget = null
                }
            }
            Shortcut {
                sequence: "Escape"
                enabled: window.replyTarget !== null && !settings.opened && !switcher.opened && !imageViewer.opened
                onActivated: window.replyTarget = null
            }
            RowLayout {
                visible: chat.selected !== ""
                Layout.fillWidth: true
                Layout.margins: 16
                ScrollView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.max(44, Math.min(168, composer.implicitHeight))
                    contentWidth: availableWidth
                    ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
                    ScrollBar.vertical.policy: ScrollBar.AsNeeded

                    TextArea {
                        id: composer
                        leftPadding: 12
                        rightPadding: 16
                        topPadding: 10
                        bottomPadding: 10
                        wrapMode: TextEdit.Wrap
                        textFormat: TextEdit.PlainText
                        background: Rectangle {
                            radius: 9
                            color: window.fieldSurface
                            border.color: composer.activeFocus ? window.accent : window.divider
                        }
                        placeholderText: chat.roomName ? "Message " + chat.roomName : "Write a message"
                        Accessible.name: placeholderText
                        enabled: chat.ready

                        Keys.onPressed: event => {
                            const enter = event.key === Qt.Key_Return || event.key === Qt.Key_Enter
                            if (!enter || composer.inputMethodComposing
                                    || (event.modifiers & ~Qt.KeypadModifier) !== Qt.NoModifier)
                                return

                            event.accepted = true
                            if (!event.isAutoRepeat && sendButton.enabled)
                                sendButton.clicked()
                        }
                    }
                }
                Button {
                    id: sendButton
                    text: "Send"
                    enabled: chat.ready && composer.text.trim().length > 0
                    onClicked: {
                        if (chat.send(composer.text, window.replyTarget?.eventId ?? "")) {
                            composer.clear()
                            window.replyTarget = null
                        }
                    }
                }
            }
        }
    }
}
