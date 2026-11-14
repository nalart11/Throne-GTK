import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: Model

    @State private var baseline: [String: Any] = [:]
    @State private var draft: [String: Any] = [:]
    @State private var routeRules: [RouteRuleDraft] = []
    @State private var loaded = false
    @State private var dirty = false
    @State private var routeRulesValid = true
    @State private var saving = false
    @State private var showingRoutes = false
    @State private var validationError: String?

    private var ru: Bool { model.language == "ru" }
    private func tr(_ ruText: String, _ enText: String) -> String { model.tr(ruText, enText) }

    var body: some View {
        VStack(spacing: 0) {
            if !loaded && !model.ready {
                ProgressView(tr("Загрузка настроек…", "Loading settings…"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                Form {
                    generalSection
                    proxySection
                    dnsSection
                    routingSection
                    testsAndSelectionSection
                    coreSection
                    subscriptionSection
                    privacyAndWindowSection
                }
                .formStyle(.grouped)
                .scrollContentBackground(.hidden)
            }

            footer
        }
        .onAppear(perform: loadIfNeeded)
        .onChange(of: model.ready) { _ in loadIfNeeded() }
        .sheet(isPresented: $showingRoutes) {
            NavigationStack {
                RoutingEditor(rules: routeBinding, ru: ru, onImported: importedRules)
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            Button(tr("Готово", "Done")) { showingRoutes = false }
                        }
                    }
            }
            .frame(minWidth: 650, minHeight: 560)
        }
        .alert(tr("Не удалось сохранить настройки", "Could not save settings"), isPresented: Binding(
            get: { validationError != nil },
            set: { if !$0 { validationError = nil } }
        )) {
            Button(tr("OK", "OK"), role: .cancel) { validationError = nil }
        } message: {
            Text(validationError ?? "")
        }
    }

    private var generalSection: some View {
        Section(tr("Основное", "General")) {
            Picker(tr("Язык", "Language"), selection: stringBinding("language")) {
                Text("Русский").tag("ru")
                Text("English").tag("en")
            }

            Picker(tr("Режим", "Mode"), selection: stringBinding("mode")) {
                Text(tr("Прокси", "Proxy")).tag("proxy")
                Text("VPN").tag("vpn")
            }
            Text(tr(
                "Сохраняются настройки для следующего подключения. Текущий режим не меняется; VPN включается только после отдельного подтверждения.",
                "Settings are saved for the next connection. The current mode does not change; VPN starts only after separate confirmation."
            ))
            .font(.caption)
            .foregroundStyle(.secondary)

            numberField("mixed_port", title: tr("Порт прокси", "Proxy port"), range: 1024...65535)
        }
    }

    private var proxySection: some View {
        Section(tr("Прокси и туннель", "Proxy & tunnel")) {
            Toggle(tr("Доступ из локальной сети", "Allow LAN access"), isOn: boolBinding("allow_lan"))
            Toggle(tr("Определять протокол и домен", "Detect protocol and domain"), isOn: boolBinding("sniffing"))
            Picker(tr("Сетевой стек", "Network stack"), selection: stringBinding("tun_stack")) {
                Text(tr("Смешанный", "Mixed")).tag("mixed")
                Text(tr("Системный", "System")).tag("system")
                Text("gVisor").tag("gvisor")
            }
            numberField("tun_mtu", title: "MTU", range: 576...65535)
            Toggle(tr("Строгая маршрутизация", "Strict routing"), isOn: boolBinding("tun_strict_route"))
            textField("tun_ipv4_cidr", title: "IPv4 CIDR")
            textField("tun_ipv6_cidr", title: "IPv6 CIDR")
            Toggle(tr("IPv6 в туннеле", "IPv6 in tunnel"), isOn: boolBinding("tun_ipv6"))
        }
    }

    private var dnsSection: some View {
        Section("DNS") {
            Toggle(tr("Встроенная служба DNS", "Built-in DNS service"), isOn: boolBinding("dns_enabled"))
            textField("dns_remote", title: tr("Через прокси", "Remote resolver"))
            textField("dns_direct", title: tr("Напрямую", "Direct resolver"))
            Picker(tr("Стратегия разрешения", "Resolution strategy"), selection: stringBinding("dns_strategy")) {
                Text(tr("Как есть", "As is")).tag("as_is")
                Text(tr("Сначала IPv4", "Prefer IPv4")).tag("prefer_ipv4")
                Text(tr("Сначала IPv6", "Prefer IPv6")).tag("prefer_ipv6")
                Text(tr("Только IPv4", "IPv4 only")).tag("ipv4_only")
                Text(tr("Только IPv6", "IPv6 only")).tag("ipv6_only")
            }
            Toggle(tr("Маршрутизировать DNS-запросы", "Route DNS queries"), isOn: boolBinding("dns_routing"))
            Toggle(tr("Подставные IP-адреса", "Fake IP addresses"), isOn: boolBinding("fakedns"))
            Toggle(tr("Обходить прокси для локальных адресов", "Bypass proxy for local addresses"), isOn: boolBinding("bypass_private"))
        }
    }

    private var routingSection: some View {
        Section(tr("Маршрутизация", "Routing")) {
            Button {
                showingRoutes = true
            } label: {
                HStack {
                    Label(tr("Правила маршрутизации", "Routing rules"), systemImage: "arrow.triangle.branch")
                    Spacer()
                    Text(ruleCount)
                        .foregroundStyle(.secondary)
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            Text(tr(
                "Порядок правил важен. Дополнительный JSON-фрагмент сохраняется отдельно.",
                "Rule order matters. The additional JSON fragment is stored separately."
            ))
            .font(.caption)
            .foregroundStyle(.secondary)

            TextEditor(text: stringBinding("custom_route_rules"))
                .font(.system(.caption, design: .monospaced))
                .frame(minHeight: 100)
                .accessibilityLabel(tr("Свои правила в формате sing-box", "Custom sing-box routing rules"))
        }
    }

    private var testsAndSelectionSection: some View {
        Section(tr("Проверки и автовыбор", "Tests & automatic selection")) {
            textField("test_url", title: tr("Адрес для проверки", "Test URL"))
            numberField("test_timeout_ms", title: tr("Таймаут проверки (мс)", "Test timeout (ms)"), range: 500...30000)
            numberField("test_concurrency", title: tr("Одновременных проверок", "Concurrent tests"), range: 1...128)
            Toggle(tr("Измерять загрузку", "Measure download speed"), isOn: boolBinding("speed_test_download"))
            Toggle(tr("Измерять отдачу", "Measure upload speed"), isOn: boolBinding("speed_test_upload"))
            numberField("speed_test_timeout_ms", title: tr("Длительность замера (мс)", "Speed test duration (ms)"), range: 2000...60000)
            numberField("auto_interval_secs", title: tr("Интервал проверки серверов (сек)", "Server retest interval (sec)"), range: 30...3600)
            numberField("auto_tolerance_ms", title: tr("Запас для переключения (мс)", "Switching margin (ms)"), range: 0...2000)
            Toggle(tr("Распределять подключения между серверами", "Balance connections across servers"), isOn: boolBinding("auto_balance"))
        }
    }

    private var coreSection: some View {
        Section(tr("Ядро и диагностика", "Core & diagnostics")) {
            Toggle(tr("Считать трафик и соединения", "Track traffic and connections"), isOn: boolBinding("stats_enabled"))
            Toggle(tr("Мультиплексирование", "Multiplexing"), isOn: boolBinding("mux_enabled"))
            Picker(tr("Протокол мультиплексирования", "Multiplexing protocol"), selection: stringBinding("mux_protocol")) {
                Text("h2mux").tag("h2mux")
                Text("smux").tag("smux")
                Text("yamux").tag("yamux")
            }
            numberField("mux_max_streams", title: tr("Потоков в соединении", "Streams per connection"), range: 1...1024)
            Picker(tr("Подробность журнала", "Log verbosity"), selection: stringBinding("log_level")) {
                Text("trace").tag("trace")
                Text("debug").tag("debug")
                Text("info").tag("info")
                Text(tr("Предупреждения", "Warnings")).tag("warn")
                Text(tr("Ошибки", "Errors")).tag("error")
            }
        }
    }

    private var subscriptionSection: some View {
        Section(tr("Подписки", "Subscriptions")) {
            Toggle(tr("Сохранять предпочтение автообновления", "Store auto-update preference"), isOn: boolBinding("sub_auto_update"))
            Text(tr(
                "Сейчас это только сохранённое предпочтение: планировщик автоматического обновления ещё не подключён. Подписку можно обновить вручную в списке групп.",
                "This is a stored preference only: automatic subscription scheduling is not implemented yet. Refresh a subscription manually from the group list."
            ))
            .font(.caption)
            .foregroundStyle(.secondary)
            numberField("sub_auto_update_minutes", title: tr("Интервал предпочтения (мин)", "Preferred interval (min)"), range: 10...10080)
            textField("sub_user_agent", title: "User-Agent")
            Toggle(tr("Отправлять HWID при обновлении", "Send HWID when refreshing"), isOn: boolBinding("sub_send_hwid"))
            textField("sub_custom_hwid_params", title: tr("Параметры HWID", "HWID parameters"))
        }
    }

    private var privacyAndWindowSection: some View {
        Section(tr("Окно", "Window")) {
            Toggle(tr("Прятать в значок при закрытии", "Hide to menu bar on close"), isOn: boolBinding("close_to_tray"))
            Toggle(tr("Запускать свёрнутым", "Start minimized"), isOn: boolBinding("start_minimized"))
            Toggle(tr("Подключаться при запуске", "Connect on launch"), isOn: boolBinding("connect_last_on_start"))
        }
    }

    private var footer: some View {
        HStack {
            if dirty {
                Text(tr("Есть несохранённые изменения", "Unsaved changes"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                Text(tr("Изменения применяются только после сохранения", "Changes apply only after saving"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button(tr("Вернуть", "Revert"), action: revert)
                .disabled(!dirty || saving)
            Button {
                save()
            } label: {
                if saving { ProgressView().controlSize(.small) }
                else { Text(tr("Сохранить", "Save")) }
            }
            .buttonStyle(.borderedProminent)
            .disabled(!dirty || saving)
            .keyboardShortcut(.defaultAction)
        }
        .padding(.horizontal, 22)
        .padding(.vertical, 12)
        .background(.bar)
    }

    private var ruleCount: String {
        let enabled = routeRules.filter(\.enabled).count
        return "\(enabled)/\(routeRules.count)"
    }

    private var routeBinding: Binding<[RouteRuleDraft]> {
        Binding(
            get: { routeRules },
            set: {
                routeRules = $0
                draft["route_rules"] = RouteRuleDraft.encode($0)
                dirty = true
            }
        )
    }

    private func loadIfNeeded() {
        guard model.ready, !loaded else { return }
        baseline = model.settings
        draft = baseline
        for key in numberKeys {
            draft[key] = stringValue(baseline[key])
        }
        do {
            routeRules = try RouteRuleDraft.decode(baseline["route_rules"])
        } catch {
            routeRulesValid = false
            validationError = tr("Сохранённые правила имеют неверный формат и не будут заменены.", "Saved routing rules are malformed and will not be overwritten.")
            routeRules = []
        }
        loaded = true
    }

    private func revert() {
        draft = baseline
        for key in numberKeys { draft[key] = stringValue(baseline[key]) }
        do { routeRules = try RouteRuleDraft.decode(baseline["route_rules"]); routeRulesValid = true }
        catch { validationError = error.localizedDescription; return }
        dirty = false
        validationError = nil
    }

    private func importedRules(_ value: [[String: Any]]) {
        let encoded = value
        do { routeRules = try RouteRuleDraft.decode(encoded) }
        catch { validationError = error.localizedDescription; return }
        routeRulesValid = true
        draft["route_rules"] = encoded
        baseline["route_rules"] = encoded
    }

    private func save() {
        guard let patch = makePatch() else { return }
        guard !patch.isEmpty else { dirty = false; return }
        saving = true
        validationError = nil
        Task { @MainActor in
            do {
                _ = try await model.perform("settings_patch", ["patch": patch])
                baseline = model.settings
                draft = baseline
                for key in numberKeys { draft[key] = stringValue(baseline[key]) }
                routeRules = try RouteRuleDraft.decode(baseline["route_rules"])
                dirty = false
            } catch {
                validationError = error.localizedDescription
            }
            saving = false
        }
    }

    private let numberKeys: Set<String> = [
        "mixed_port", "tun_mtu", "mux_max_streams", "test_timeout_ms", "test_concurrency",
        "speed_test_timeout_ms", "auto_interval_secs", "auto_tolerance_ms",
        "sub_auto_update_minutes"
    ]

    private let stringKeys: [String] = [
        "mode", "tun_stack", "tun_ipv4_cidr", "tun_ipv6_cidr", "dns_remote", "dns_direct",
        "dns_strategy", "custom_route_rules", "log_level", "mux_protocol", "test_url",
        "sub_user_agent", "sub_custom_hwid_params", "language"
    ]

    private let boolKeys: [String] = [
        "allow_lan", "sniffing", "tun_strict_route", "tun_ipv6", "dns_enabled", "dns_routing",
        "fakedns", "bypass_private", "stats_enabled", "mux_enabled", "speed_test_download",
        "speed_test_upload", "auto_balance", "sub_auto_update", "sub_send_hwid", "close_to_tray",
        "start_minimized", "connect_last_on_start"
    ]

    private func makePatch() -> [String: Any]? {
        var patch: [String: Any] = [:]
        for key in stringKeys {
            let value = draft[key] as? String ?? ""
            if !equal(value, baseline[key]) { patch[key] = value }
        }
        for key in boolKeys {
            let value = draft[key] as? Bool ?? false
            if !equal(value, baseline[key]) { patch[key] = value }
        }
        for key in numberKeys {
            let raw = draft[key] as? String ?? ""
            if raw == stringValue(baseline[key]) { continue }
            guard let value = Int64(raw) else {
                validationError = tr("Введите целое число для поля «\(key)».", "Enter a whole number for “\(key)”.")
                return nil
            }
            guard validRange(value, for: key) else {
                validationError = rangeMessage(for: key)
                return nil
            }
            let normalized: Any = ["auto_interval_secs", "sub_auto_update_minutes"].contains(key) ? value : Int(value)
            if !equal(normalized, baseline[key]) { patch[key] = normalized }
        }

        if routeRulesValid, let draftRules = draft["route_rules"], !equal(draftRules, baseline["route_rules"]) {
            let encodedRules = RouteRuleDraft.encode(routeRules)
            if !equal(encodedRules, baseline["route_rules"]) { patch["route_rules"] = encodedRules }
        }
        let customRules = draft["custom_route_rules"] as? String ?? ""
        if patch["custom_route_rules"] != nil, !customRules.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            guard let data = customRules.data(using: .utf8), (try? JSONSerialization.jsonObject(with: data)) != nil else {
                validationError = tr("Свои правила должны быть корректным JSON.", "Custom routing rules must be valid JSON.")
                return nil
            }
        }
        if !equal(customRules, baseline["custom_route_rules"]) { patch["custom_route_rules"] = customRules }

        for rule in routeRules where patch["route_rules"] != nil && rule.enabled {
            if !rule.conditions.contains(where: { $0.values.contains(where: { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) }) {
                validationError = tr("У включённого правила должно быть хотя бы одно значение.", "Each enabled rule needs at least one condition value.")
                return nil
            }
        }
        return patch
    }

    private func validRange(_ value: Int64, for key: String) -> Bool {
        switch key {
        case "mixed_port": return (1024...65535).contains(value)
        case "tun_mtu": return (576...65535).contains(value)
        case "mux_max_streams": return (1...1024).contains(value)
        case "test_timeout_ms": return (500...30000).contains(value)
        case "test_concurrency": return (1...128).contains(value)
        case "speed_test_timeout_ms": return (2000...60000).contains(value)
        case "auto_interval_secs": return (30...3600).contains(value)
        case "auto_tolerance_ms": return (0...2000).contains(value)
        case "sub_auto_update_minutes": return (10...10080).contains(value)
        default: return false
        }
    }

    private func rangeMessage(for key: String) -> String {
        let range: String
        switch key {
        case "mixed_port": range = "1024–65535"
        case "tun_mtu": range = "576–65535"
        case "mux_max_streams": range = "1–1024"
        case "test_timeout_ms": range = "500–30000"
        case "test_concurrency": range = "1–128"
        case "speed_test_timeout_ms": range = "2000–60000"
        case "auto_interval_secs": range = "30–3600"
        case "auto_tolerance_ms": range = "0–2000"
        case "sub_auto_update_minutes": range = "10–10080"
        default: range = "0+"
        }
        return tr("Значение «\(key)» должно быть в диапазоне \(range).", "“\(key)” must be in the range \(range).")
    }

    private func equal(_ lhs: Any, _ rhs: Any?) -> Bool {
        guard let rhs,
              let leftData = try? JSONSerialization.data(withJSONObject: [lhs], options: [.sortedKeys]),
              let rightData = try? JSONSerialization.data(withJSONObject: [rhs], options: [.sortedKeys]) else { return false }
        return leftData == rightData
    }

    private func stringValue(_ value: Any?) -> String {
        guard let value else { return "" }
        if let number = value as? NSNumber { return number.stringValue }
        return String(describing: value)
    }

    private func boolBinding(_ key: String) -> Binding<Bool> {
        Binding(
            get: { draft[key] as? Bool ?? false },
            set: { draft[key] = $0; dirty = true }
        )
    }

    private func stringBinding(_ key: String) -> Binding<String> {
        Binding(
            get: { draft[key] as? String ?? "" },
            set: { draft[key] = $0; dirty = true }
        )
    }

    private func textField(_ key: String, title: String) -> some View {
        TextField(title, text: stringBinding(key))
            .textFieldStyle(.roundedBorder)
    }

    private func numberField(_ key: String, title: String, range: ClosedRange<Int>) -> some View {
        TextField(title, text: stringBinding(key))
            .textFieldStyle(.roundedBorder)
            .frame(maxWidth: 180)
            .accessibilityHint(tr("Допустимый диапазон: \(range.lowerBound)–\(range.upperBound)", "Allowed range: \(range.lowerBound)–\(range.upperBound)"))
    }
}
