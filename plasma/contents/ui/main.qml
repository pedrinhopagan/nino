pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.plasma5support as P5Support

PlasmoidItem {
    id: root

    readonly property string dir: '"$HOME/.config/nino"'
    readonly property string listFlags: "ls -1 " + dir + " 2>/dev/null; true"
    readonly property string listDevices: "gpu-screen-recorder --list-audio-devices"
    property var flags: []
    property var devices: null
    readonly property bool system: flags.indexOf("audio-pc") >= 0
    readonly property bool mic: flags.indexOf("microfone") >= 0

    Plasmoid.status: PlasmaCore.Types.PassiveStatus
    Plasmoid.icon: "nino"
    toolTipMainText: "Nino"
    toolTipSubText: system && mic ? "Grava o som do PC e o microfone"
        : system ? "Grava o som do PC"
        : mic ? "Grava o microfone"
        : "Grava sem áudio"

    function check() {
        shell.connectSource(listFlags)
        shell.connectSource(listDevices)
    }

    function set(name, on) {
        const path = dir + "/" + name
        shell.connectSource(on ? "mkdir -p " + dir + " && touch " + path : "rm -f " + path)
    }

    function available(device) {
        return devices === null || devices.indexOf(device) >= 0
    }

    onExpandedChanged: if (expanded) check()
    Component.onCompleted: check()

    P5Support.DataSource {
        id: shell

        engine: "executable"
        onNewData: (source, data) => {
            disconnectSource(source)
            const lines = (data.stdout || "").split("\n").filter(line => line.length > 0)
            if (source === root.listFlags)
                root.flags = lines
            else if (source === root.listDevices)
                root.devices = data["exit code"] === 0 ? lines.map(line => line.split("|")[0]) : null
            else
                connectSource(root.listFlags)
        }
    }

    component Option: RowLayout {
        id: option

        required property string icon
        required property string title
        required property string detail
        required property string flag
        required property bool on
        property bool present: true

        Layout.fillWidth: true
        spacing: 12
        enabled: present
        opacity: present ? 1 : 0.5

        Kirigami.Icon {
            Layout.alignment: Qt.AlignTop
            Layout.topMargin: 1
            implicitWidth: Kirigami.Units.iconSizes.small
            implicitHeight: Kirigami.Units.iconSizes.small
            source: option.icon
            color: Kirigami.Theme.textColor
            isMask: true
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            PlasmaComponents.Label {
                Layout.fillWidth: true
                text: option.title
                elide: Text.ElideRight
            }

            PlasmaComponents.Label {
                Layout.fillWidth: true
                text: option.present ? option.detail : "Nenhum dispositivo encontrado."
                color: Qt.alpha(Kirigami.Theme.textColor, 0.7)
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                wrapMode: Text.WordWrap
            }
        }

        PlasmaComponents.Switch {
            Layout.alignment: Qt.AlignVCenter
            checked: option.on && option.present
            Accessible.name: "Gravar " + option.title.toLowerCase()
            onToggled: root.set(option.flag, checked)
        }
    }

    fullRepresentation: Item {
        Layout.minimumWidth: 320
        Layout.preferredWidth: 320
        Layout.maximumWidth: 320
        Layout.minimumHeight: layout.implicitHeight
        Layout.preferredHeight: layout.implicitHeight

        ColumnLayout {
            id: layout

            anchors.top: parent.top
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 24
            anchors.rightMargin: 24
            spacing: 0

            PlasmaComponents.Label {
                Layout.fillWidth: true
                Layout.topMargin: 16
                Layout.bottomMargin: 12
                text: "Áudio da gravação"
                color: Qt.alpha(Kirigami.Theme.textColor, 0.7)
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                font.weight: Font.DemiBold
            }

            Option {
                icon: "audio-speakers-symbolic"
                title: "Som do PC"
                detail: "Tudo que toca na saída padrão."
                flag: "audio-pc"
                on: root.system
                present: root.available("default_output")
            }

            Option {
                Layout.topMargin: 16
                icon: "audio-input-microphone-symbolic"
                title: "Microfone"
                detail: "Sua voz, com redução de chiado no vídeo final."
                flag: "microfone"
                on: root.mic
                present: root.available("default_input")
            }

            Kirigami.Separator {
                Layout.fillWidth: true
                Layout.topMargin: 16
                opacity: 0.5
            }

            PlasmaComponents.Label {
                Layout.fillWidth: true
                Layout.topMargin: 12
                Layout.bottomMargin: 16
                text: "Vale a partir da próxima gravação. Na revisão você ouve cada faixa e escolhe quais entram no vídeo."
                color: Qt.alpha(Kirigami.Theme.textColor, 0.55)
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                wrapMode: Text.WordWrap
            }
        }
    }
}
