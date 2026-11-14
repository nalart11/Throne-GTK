import AppKit
import Combine
import SwiftUI

@MainActor final class Model: ObservableObject {
    @Published var language = "ru"
    var settings: [String: Any] = [:]
    func tr(_ ru: String, _ en: String) -> String { language == "en" ? en : ru }
    func shutdown(reply: @escaping (Bool) -> Void) { reply(true) }
}

@main struct MenuLocalizationSmoke {
    @MainActor static func main() throws {
        _ = NSApplication.shared
        let main = NSMenu(title: "Main")
        let appMenu = NSMenu(title: "App")
        let editMenu = NSMenu(title: "Edit")
        let viewMenu = NSMenu(title: "SwiftUI Commands")
        let windowMenu = NSMenu(title: "Window")
        let helpMenu = NSMenu(title: "Help")
        let fileMenu = NSMenu(title: "File")
        let systemAppleMenu = NSMenu(title: "Apple")
        editMenu.autoenablesItems = false
        fileMenu.autoenablesItems = false

        let target = NSObject()
        func item(_ title: String, _ action: String, key: String = "") -> NSMenuItem {
            let result = NSMenuItem(title: title, action: Selector((action)), keyEquivalent: key)
            result.target = target
            result.isEnabled = false
            return result
        }
        func root(_ title: String, _ menu: NSMenu) {
            let item = NSMenuItem(title: title, action: nil, keyEquivalent: "")
            item.submenu = menu
            main.addItem(item)
        }

        let undo = item("Undo", "undo:", key: "z")
        editMenu.addItem(undo)
        viewMenu.addItem(item("Platform command", "swiftUIGeneratedViewCommand:"))
        windowMenu.addItem(item("Minimize", "performMiniaturize:"))
        windowMenu.addItem(item("Zoom", "performZoom:"))
        helpMenu.addItem(item("Help", "showHelp:"))
        let close = item("Close", "performClose:", key: "w")
        fileMenu.addItem(close)
        systemAppleMenu.addItem(item("About This Mac", "aboutThisMac:"))
        appMenu.addItem(item("Hide Nolurix", "hide:"))
        appMenu.addItem(item("Quit Nolurix", "terminate:", key: "q"))
        root("Nolurix", appMenu)
        root("Edit", editMenu)
        root("Вид", viewMenu)
        root("Window", windowMenu)
        root("Help", helpMenu)
        root("File", fileMenu)
        root("Apple", systemAppleMenu)

        let original = [undo, close].map {
            ($0, $0.action, $0.target, $0.keyEquivalent, $0.keyEquivalentModifierMask, $0.isEnabled)
        }
        let originalAutoEnabling = [appMenu, editMenu, viewMenu, windowMenu, helpMenu, fileMenu, systemAppleMenu].map(\.autoenablesItems)
        for language in ["ru", "en", "ru"] {
            NativeMenuLocalizer.localize(mainMenu: main, helpMenu: helpMenu, windowsMenu: windowMenu, language: language)
            let expected = language == "en"
            try require(main.items[1].title == (expected ? "Edit" : "Правка"), "Edit root title")
            try require(editMenu.title == main.items[1].title, "Edit submenu title")
            try require(main.items[2].title == (expected ? "View" : "Вид"), "View root title")
            try require(viewMenu.title == (expected ? "View" : "Вид"), "View submenu title")
            try require(main.items[3].title == (expected ? "Window" : "Окно"), "Window root title")
            try require(main.items[4].title == (expected ? "Help" : "Справка"), "Help root title")
            try require(main.items[5].title == "File", "File menu misclassified as Window")
            try require(main.items[6].title == "Apple" && systemAppleMenu.title == "Apple", "system Apple menu changed")
            try require(undo.title == (expected ? "Undo" : "Отменить"), "Edit command title")
            try require(close.title == (expected ? "Close Window" : "Закрыть окно"), "File close command title")
            for (item, action, itemTarget, key, modifiers, enabled) in original {
                try require(item.action == action && item.target === itemTarget && item.keyEquivalent == key && item.keyEquivalentModifierMask == modifiers && item.isEnabled == enabled,
                            "menu behavior changed while relabeling")
            }
            try require([appMenu, editMenu, viewMenu, windowMenu, helpMenu, fileMenu, systemAppleMenu].map(\.autoenablesItems) == originalAutoEnabling,
                        "menu autoenabling changed while relabeling")
        }
        print("PASS: AppKit menu RU→EN→RU relabeling; roles, File/Apple preservation, selectors, targets, shortcuts and enabled state")
    }

    @MainActor private static func require(_ condition: Bool, _ message: String) throws {
        if !condition { throw NSError(domain: "MenuLocalizationSmoke", code: 1, userInfo: [NSLocalizedDescriptionKey: message]) }
    }
}
