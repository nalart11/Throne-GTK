import AppKit
import SwiftUI
import Combine

@MainActor func throneResource(_ name: String, extension ext: String) -> URL? {
    if let url = Bundle.main.url(forResource: name, withExtension: ext) { return url }
    let executable = URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL
    let url = executable.deletingLastPathComponent().appendingPathComponent("Resources").appendingPathComponent("\(name).\(ext)")
    return FileManager.default.fileExists(atPath: url.path) ? url : nil
}

enum NativeMenuLocalizer {
    static func localize(mainMenu: NSMenu, helpMenu: NSMenu?, windowsMenu: NSMenu?, language: String) {
        let ru = language != "en"
        for root in mainMenu.items {
            guard let menu = root.submenu else { continue }
            let title: String?
            switch role(of: menu, rootTitle: root.title, helpMenu: helpMenu, windowsMenu: windowsMenu) {
            case .help: title = ru ? "Справка" : "Help"
            case .window: title = ru ? "Окно" : "Window"
            case .edit: title = ru ? "Правка" : "Edit"
            case .view: title = ru ? "Вид" : "View"
            case nil: title = nil
            }
            if let title { root.title = title; menu.title = title }
            localizeItems(in: menu, ru: ru)
        }
    }

    private enum Role { case help, window, edit, view }

    private static func role(of menu: NSMenu, rootTitle: String, helpMenu: NSMenu?, windowsMenu: NSMenu?) -> Role? {
        if menu === helpMenu || !actions(in: menu).isDisjoint(with: ["showHelp:", "searchForHelp:"]) { return .help }
        if menu === windowsMenu { return .window }
        if ["View", "Вид"].contains(rootTitle) || ["View", "Вид"].contains(menu.title) { return .view }
        let menuActions = actions(in: menu)
        let windowSignature: Set<String> = ["performMiniaturize:", "performZoom:", "arrangeInFront:"]
        if menuActions.intersection(windowSignature).count >= 2 { return .window }
        if !menuActions.isDisjoint(with: ["undo:", "redo:", "cut:", "copy:", "paste:", "selectAll:"]) { return .edit }
        if !menuActions.isDisjoint(with: ["toggleSidebar:", "toggleToolbarShown:", "enterFullScreen:", "exitFullScreen:"]) { return .view }
        return nil
    }

    private static func actions(in menu: NSMenu) -> Set<String> {
        var result = Set<String>()
        for item in menu.items {
            if let action = item.action { result.insert(NSStringFromSelector(action)) }
            if let submenu = item.submenu { result.formUnion(actions(in: submenu)) }
        }
        return result
    }

    private static func localizeItems(in menu: NSMenu, ru: Bool) {
        let titles: [String: (String, String)] = [
            "orderFrontStandardAboutPanel:": ("О программе Throne GTK", "About Throne GTK"),
            "hide:": ("Скрыть Throne GTK", "Hide Throne GTK"),
            "hideOtherApplications:": ("Скрыть остальные", "Hide Others"),
            "unhideAllApplications:": ("Показать все", "Show All"),
            "terminate:": ("Выйти из Throne GTK", "Quit Throne GTK"),
            "undo:": ("Отменить", "Undo"), "redo:": ("Повторить", "Redo"),
            "cut:": ("Вырезать", "Cut"), "copy:": ("Копировать", "Copy"),
            "paste:": ("Вставить", "Paste"), "pasteAsPlainText:": ("Вставить и подобрать стиль", "Paste and Match Style"),
            "delete:": ("Удалить", "Delete"), "selectAll:": ("Выбрать всё", "Select All"),
            "toggleSidebar:": ("Показать или скрыть боковую панель", "Toggle Sidebar"),
            "toggleToolbarShown:": ("Показать или скрыть панель инструментов", "Show/Hide Toolbar"),
            "enterFullScreen:": ("На весь экран", "Enter Full Screen"),
            "exitFullScreen:": ("Выйти из полноэкранного режима", "Exit Full Screen"),
            "performMiniaturize:": ("Свернуть", "Minimize"), "performZoom:": ("Увеличить", "Zoom"),
            "performClose:": ("Закрыть окно", "Close Window"), "arrangeInFront:": ("Расположить все окна на переднем плане", "Bring All to Front")
        ]
        for item in menu.items {
            if let action = item.action, let pair = titles[NSStringFromSelector(action)] {
                item.title = ru ? pair.0 : pair.1
            }
            if let submenu = item.submenu { localizeItems(in: submenu, ru: ru) }
        }
    }
}

@MainActor final class AppLifecycle: NSObject, NSApplicationDelegate, NSWindowDelegate {
    var model: Model? { didSet { observeLanguage(); updateMenu() } }
    private weak var mainWindow: NSWindow?
    private var statusItem: NSStatusItem?
    private var languageSubscription: AnyCancellable?
    private var menuTrackingObserver: NSObjectProtocol?
    private var terminating = false
    func applicationDidFinishLaunching(_ notification: Notification) {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem = item
        if let url = throneResource("ThroneSymbolic", extension: "png"), let image = NSImage(contentsOf: url) {
            let maxSize = NSSize(width: 20, height: 20)
            let scale = min(maxSize.width / image.size.width, maxSize.height / image.size.height)
            image.size = NSSize(width: image.size.width * scale, height: image.size.height * scale)
            image.isTemplate = true; item.button?.image = image
        } else {
            // A diagnostic, never a substitute SF-symbol logo.
            item.button?.title = "!"
            NSLog("Throne: missing Resources/ThroneSymbolic.png; status logo unavailable")
        }
        item.button?.toolTip = "Throne GTK"
        updateMenu()
        localizeMainMenu()
        menuTrackingObserver = NotificationCenter.default.addObserver(
            forName: NSMenu.didBeginTrackingNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in self?.localizeMainMenu() }
        }
    }
    private func observeLanguage() {
        languageSubscription = model?.$language.sink { [weak self] _ in
            Task { @MainActor [weak self] in self?.updateMenu() }
            Task { @MainActor [weak self] in
                await Task.yield() // First attempt after SwiftUI refreshes command labels.
                self?.localizeMainMenu()
            }
        }
    }
    private func updateMenu() {
        let menu = NSMenu()
        let show = NSMenuItem(title: model?.tr("Показать Throne GTK", "Show Throne GTK") ?? "Показать Throne GTK", action: #selector(showWindow), keyEquivalent: "")
        show.target = self; menu.addItem(show); menu.addItem(.separator())
        let quit = NSMenuItem(title: model?.tr("Выйти", "Quit") ?? "Выйти", action: #selector(quit), keyEquivalent: "q")
        quit.target = self; menu.addItem(quit); statusItem?.menu = menu
    }
    func localizeMainMenu() {
        guard let mainMenu = NSApp.mainMenu else { return }
        NativeMenuLocalizer.localize(mainMenu: mainMenu, helpMenu: NSApp.helpMenu, windowsMenu: NSApp.windowsMenu, language: model?.language ?? "ru")
    }
    func registerMainWindow(_ window: NSWindow) {
        guard mainWindow !== window else { return }
        mainWindow = window; window.identifier = NSUserInterfaceItemIdentifier("throne-main"); window.delegate = self
    }
    @objc func showWindow() {
        guard let mainWindow else { NSLog("Throne: main window not registered yet"); return }
        if mainWindow.isMiniaturized { mainWindow.deminiaturize(nil) }
        mainWindow.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
    }
    @objc private func quit() { NSApp.terminate(nil) }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool { showWindow(); return false }
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard sender === mainWindow else { return true }
        if model?.settings["close_to_tray"] as? Bool == true { sender.orderOut(nil) }
        else { Task { @MainActor in NSApp.terminate(nil) } }
        return false
    }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard !terminating else { return .terminateLater }
        guard let model else { return .terminateNow }
        terminating = true
        finishTermination(sender, model: model)
        return .terminateLater
    }
    private func finishTermination(_ sender: NSApplication, model: Model) {
        model.shutdown { [weak self] success in
            guard let self else { return }
            if success { sender.reply(toApplicationShouldTerminate: true); return }
            let alert = NSAlert()
            alert.messageText = model.tr("Не удалось безопасно закрыть Throne GTK", "Throne GTK could not close safely")
            alert.informativeText = model.tr("Остановка ещё не подтверждена. Повторите выход с тем же подключением к мосту или оставьте окно диагностики открытым.", "Shutdown is unconfirmed. Retry using the same bridge handle, or keep the diagnostic window open.")
            alert.addButton(withTitle: model.tr("Повторить", "Retry"))
            alert.addButton(withTitle: model.tr("Оставить открытым", "Keep Open"))
            if alert.runModal() == .alertFirstButtonReturn { self.finishTermination(sender, model: model) }
            else { self.terminating = false; sender.reply(toApplicationShouldTerminate: false); self.showWindow() }
        }
    }
}

// Only the main scene installs this accessor. Settings/alerts keep their own delegates.
@MainActor struct MainWindowAccessor: NSViewRepresentable {
    let lifecycle: AppLifecycle
    func makeNSView(context: Context) -> NSView { WindowProbe(lifecycle: lifecycle) }
    func updateNSView(_ view: NSView, context: Context) {
        if let window = view.window { lifecycle.registerMainWindow(window) }
    }
    final class WindowProbe: NSView {
        let lifecycle: AppLifecycle
        init(lifecycle: AppLifecycle) { self.lifecycle = lifecycle; super.init(frame: .zero) }
        required init?(coder: NSCoder) { fatalError("init(coder:) unavailable") }
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); if let window { lifecycle.registerMainWindow(window) } }
    }
}
