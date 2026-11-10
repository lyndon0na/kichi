import QtQuick
import QtTest
import "../components"

// 空结果状态的建议：索引关闭时要说清「只搜了当前目录」并给开启入口；
// 索引已在工作或正在建库时不要重复劝开启。
TestCase {
    name: "SearchEmptyState"
    when: windowShown
    width: 480
    height: 320

    QtObject {
        id: fakeBackend

        property bool searchActive: true
        property bool searchSettled: true
        property var searchResults: []
        property string searchQuery: "电影"
        property string searchSource: "fallback"
        property bool localIndexEnabled: false
        property bool indexBuilding: false
        property int enableCalls: 0

        function setLocalIndexEnabled(enabled) {
            enableCalls += 1
            localIndexEnabled = enabled
        }
    }

    Component {
        id: stateComponent

        SearchEmptyState {
            backend: fakeBackend
            visible: true
            width: 400
        }
    }

    property var state: null

    function init() {
        state = stateComponent.createObject(parent)
    }

    function cleanup() {
        if (state)
            state.destroy()
        state = null
        fakeBackend.searchSource = "fallback"
        fakeBackend.localIndexEnabled = false
        fakeBackend.indexBuilding = false
        fakeBackend.enableCalls = 0
    }

    function advice() {
        return findChild(state, "searchEmptyStateAdvice")
    }

    function enableButton() {
        return findChild(state, "enableLocalIndexButton")
    }

    function test_index_off_explains_scope_and_offers_enable() {
        verify(state.visible, "空结果时应显示排查建议")
        verify(enableButton() !== null, "缺少开启索引入口")
        verify(enableButton().visible, "索引关闭时应提供开启按钮")
        verify(advice().text.indexOf("只搜了当前文件夹") >= 0,
               "要说明这次只搜了当前目录：" + advice().text)
        verify(advice().text.indexOf("还没收录") < 0,
               "索引关闭时不能说成索引已收录不全：" + advice().text)
    }

    function test_enable_button_turns_index_on() {
        mouseClick(enableButton())
        compare(fakeBackend.enableCalls, 1)
        compare(fakeBackend.localIndexEnabled, true)
        verify(!enableButton().visible, "开关已经打开后不再重复给开启按钮")
    }

    function test_building_states_do_not_offer_enable() {
        fakeBackend.localIndexEnabled = true
        fakeBackend.indexBuilding = true
        wait(0)
        verify(!enableButton().visible, "建库中不应再提示开启")
        verify(advice().text.indexOf("正在后台建立") >= 0, "要说清建库中：" + advice().text)
    }

    function test_not_ready_states_do_not_offer_enable() {
        fakeBackend.localIndexEnabled = true
        wait(0)
        verify(!enableButton().visible, "已开启但未就绪时不应提示开启")
        verify(advice().text.indexOf("尚未就绪") >= 0, "要说清索引尚未就绪：" + advice().text)
    }

    function test_index_search_keeps_generic_advice() {
        fakeBackend.searchSource = "index"
        wait(0)
        verify(!enableButton().visible, "索引搜索不应出现开启按钮")
        verify(advice().text.indexOf("可以试试") >= 0, "索引已工作时保留通用建议：" + advice().text)
    }
}
