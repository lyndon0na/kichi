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
        property bool localIndexReady: false
        property bool indexBuilding: false
        property int enableCalls: 0
        property int clearFilterCalls: 0
        property int searchFilterActiveCount: 0
        property string searchFilterSummary: ""

        function setLocalIndexEnabled(enabled) {
            enableCalls += 1
            localIndexEnabled = enabled
        }

        function clearSearchFilters() {
            clearFilterCalls += 1
            searchFilterActiveCount = 0
            searchFilterSummary = ""
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
    property int clearRequests: 0

    function init() {
        state = stateComponent.createObject(parent)
        state.clearFiltersRequested.connect(function () {
            clearRequests += 1
        })
    }

    function cleanup() {
        if (state)
            state.destroy()
        state = null
        clearRequests = 0
        fakeBackend.searchSource = "fallback"
        fakeBackend.localIndexEnabled = false
        fakeBackend.localIndexReady = false
        fakeBackend.indexBuilding = false
        fakeBackend.enableCalls = 0
        fakeBackend.clearFilterCalls = 0
        fakeBackend.searchFilterActiveCount = 0
        fakeBackend.searchFilterSummary = ""
    }

    function advice() {
        return findChild(state, "searchEmptyStateAdvice")
    }

    function enableButton() {
        return findChild(state, "enableLocalIndexButton")
    }

    function clearButton() {
        return findChild(state, "clearSearchFiltersButton")
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

    function test_ready_index_explains_input_scope_and_enter_search() {
        fakeBackend.localIndexEnabled = true
        fakeBackend.localIndexReady = true
        wait(0)
        verify(!enableButton().visible, "索引就绪时不应出现开启按钮")
        verify(advice().text.indexOf("按回车") >= 0,
               "索引就绪时仍只筛当前目录，要提示回车才能全盘：" + advice().text)
        verify(advice().text.indexOf("尚未就绪") < 0,
               "索引就绪时不能说成尚未就绪：" + advice().text)
    }

    function test_index_search_keeps_generic_advice() {
        fakeBackend.searchSource = "index"
        wait(0)
        verify(!enableButton().visible, "索引搜索不应出现开启按钮")
        verify(advice().text.indexOf("可以试试") >= 0, "索引已工作时保留通用建议：" + advice().text)
    }

    function test_active_filters_explain_and_offer_clear() {
        fakeBackend.searchFilterActiveCount = 2
        fakeBackend.searchFilterSummary = "视频 · 最近 7 天"
        wait(0)
        verify(clearButton() !== null, "缺少清除筛选入口")
        verify(clearButton().visible, "有筛选且无结果时应提供清除按钮")
        verify(advice().text.indexOf("当前筛选条件") >= 0,
               "空态要提示可能是被筛选筛掉的：" + advice().text)
        verify(advice().text.indexOf("视频 · 最近 7 天") >= 0,
               "摘要要写进提示里：" + advice().text)
        verify(advice().text.indexOf("可以试试") < 0,
               "有筛选时先给筛选建议，不要直接给通用建议：" + advice().text)
    }

    function test_clear_button_clears_filters_and_requests_rerun() {
        fakeBackend.searchFilterActiveCount = 1
        fakeBackend.searchFilterSummary = "文件夹"
        wait(0)
        mouseClick(clearButton())
        compare(fakeBackend.clearFilterCalls, 1)
        compare(clearRequests, 1, "清除筛选后要通知文件页重跑当前关键词")
        verify(!clearButton().visible, "清除后不再显示清除入口")
    }

    function test_no_clear_entry_without_filters() {
        wait(0)
        verify(!clearButton().visible, "没有筛选时不显示清除入口")
    }
}
