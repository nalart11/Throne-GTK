import AppKit
import SwiftUI
import UniformTypeIdentifiers

enum RouteConditionKind: String, CaseIterable, Codable, Identifiable {
    case domainSuffix = "domain_suffix"
    case domain
    case domainKeyword = "domain_keyword"
    case domainRegex = "domain_regex"
    case ipCidr = "ip_cidr"
    case port
    case process
    case ruleSet = "rule_set"

    var id: String { rawValue }

    func title(ru: Bool) -> String {
        switch self {
        case .domainSuffix: return ru ? "Домен и поддомены" : "Domain and subdomains"
        case .domain: return ru ? "Домен целиком" : "Exact domain"
        case .domainKeyword: return ru ? "Часть домена" : "Domain keyword"
        case .domainRegex: return ru ? "Регулярное выражение" : "Domain expression"
        case .ipCidr: return ru ? "Адрес или подсеть" : "Address or subnet"
        case .port: return ru ? "Порт" : "Port"
        case .process: return ru ? "Процесс" : "Process"
        case .ruleSet: return ru ? "Готовый список" : "Rule set"
        }
    }

    var hint: String {
        switch self {
        case .domainSuffix: return "example.com"
        case .domain: return "example.com"
        case .domainKeyword: return "googlevideo"
        case .domainRegex: return #"^ads\..*"#
        case .ipCidr: return "10.0.0.0/8"
        case .port: return "443, 8000:8100"
        case .process: return "Firefox"
        case .ruleSet: return "geosite-ru, geoip-de"
        }
    }
}

struct RouteConditionDraft: Codable, Hashable, Identifiable {
    var id = UUID()
    var kind: RouteConditionKind = .domainSuffix
    var values: [String] = []

    private enum CodingKeys: String, CodingKey { case kind, values }

    init(kind: RouteConditionKind = .domainSuffix, values: [String] = []) {
        self.kind = kind
        self.values = values
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        kind = try values.decode(RouteConditionKind.self, forKey: .kind)
        self.values = try values.decodeIfPresent([String].self, forKey: .values) ?? []
    }
}

struct RouteRuleDraft: Codable, Hashable, Identifiable {
    var id = UUID()
    var name = ""
    var enabled = true
    var conditions: [RouteConditionDraft] = []
    var matchMode = RouteMatchMode.any
    var action = RouteActionChoice.direct

    private enum CodingKeys: String, CodingKey {
        case name, enabled, conditions, matchMode = "match_mode", action
        // Read legacy single-condition records without losing them on edit.
        case kind, values
    }

    init() {}

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        name = try container.decodeIfPresent(String.self, forKey: .name) ?? ""
        enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        matchMode = try container.decodeIfPresent(RouteMatchMode.self, forKey: .matchMode) ?? .any
        action = try container.decodeIfPresent(RouteActionChoice.self, forKey: .action) ?? .direct
        conditions = try container.decodeIfPresent([RouteConditionDraft].self, forKey: .conditions) ?? []
        if conditions.isEmpty,
           let kind = try container.decodeIfPresent(RouteConditionKind.self, forKey: .kind),
           let values = try container.decodeIfPresent([String].self, forKey: .values) {
            conditions = [RouteConditionDraft(kind: kind, values: values)]
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(name, forKey: .name)
        try container.encode(enabled, forKey: .enabled)
        try container.encode(conditions, forKey: .conditions)
        try container.encode(matchMode, forKey: .matchMode)
        try container.encode(action, forKey: .action)
    }

    static func decode(_ value: Any?) throws -> [RouteRuleDraft] {
        guard let value else { throw RouteDraftError.invalidData }
        let data = try JSONSerialization.data(withJSONObject: value)
        return try JSONDecoder().decode([RouteRuleDraft].self, from: data)
    }

    static func encode(_ rules: [RouteRuleDraft]) -> [[String: Any]] {
        guard let data = try? JSONEncoder().encode(rules),
              let value = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return [] }
        return value
    }
}

enum RouteDraftError: LocalizedError {
    case invalidData
    var errorDescription: String? { "Saved routing rules do not match the expected schema." }
}

enum RouteMatchMode: String, Codable, CaseIterable, Identifiable {
    case any
    case all
    var id: String { rawValue }
    func title(ru: Bool) -> String {
        switch self {
        case .any: return ru ? "Любое условие" : "Match any condition"
        case .all: return ru ? "Все условия" : "Match all conditions"
        }
    }
}

enum RouteActionChoice: String, Codable, CaseIterable, Identifiable {
    case proxy
    case direct
    case block
    var id: String { rawValue }
    func title(ru: Bool) -> String {
        switch self {
        case .proxy: return ru ? "Через прокси" : "Through proxy"
        case .direct: return ru ? "Напрямую" : "Direct"
        case .block: return ru ? "Блокировать" : "Block"
        }
    }
}

struct RoutingEditor: View {
    @EnvironmentObject private var model: Model
    @Binding var rules: [RouteRuleDraft]
    let ru: Bool
    let onImported: ([[String: Any]]) -> Void

    @State private var editingRule: RouteRuleDraft?
    @State private var isAddingRule = false
    @State private var importConfirmation = false
    @State private var importPath: String?
    @State private var errorMessage: String?
    @State private var importing = false

    private func tr(_ ruText: String, _ enText: String) -> String { ru ? ruText : enText }

    var body: some View {
        List {
            Section {
                if rules.isEmpty {
                    ContentUnavailableViewCompat(
                        title: tr("Правил пока нет", "No routing rules yet"),
                        detail: tr("Добавьте правило. Порядок определяет приоритет.", "Add a rule. List order determines priority."),
                        symbol: "arrow.triangle.branch"
                    )
                }
                ForEach(Array(rules.enumerated()), id: \.element.id) { index, rule in
                    ruleRow(rule, at: index)
                }
            } header: {
                Text(tr("Правила", "Rules"))
            } footer: {
                Text(tr("Срабатывает первое подходящее правило. Не совпавший трафик идёт через прокси.", "The first matching rule applies. Unmatched traffic goes through the proxy."))
            }

            Section {
                Button {
                    isAddingRule = true
                } label: {
                    Label(tr("Добавить правило", "Add rule"), systemImage: "plus")
                }
                .keyboardShortcut("n", modifiers: [.command])

                Button {
                    chooseImportFile()
                } label: {
                    Label(tr("Импортировать правила из старой базы", "Import rules from legacy database"), systemImage: "square.and.arrow.down")
                }
                .disabled(importing)

                if importing { ProgressView(tr("Импорт правил…", "Importing rules…")) }
            } header: {
                Text(tr("Инструменты", "Tools"))
            }
        }
        .navigationTitle(tr("Маршрутизация", "Routing"))
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { isAddingRule = true } label: { Label(tr("Добавить", "Add"), systemImage: "plus") }
                    .keyboardShortcut("n", modifiers: [.command])
            }
        }
        .sheet(item: $editingRule) { rule in
            RouteRuleForm(rule: rule, ru: ru) { changed in
                if let index = rules.firstIndex(where: { $0.id == rule.id }) { rules[index] = changed }
                editingRule = nil
            } onCancel: { editingRule = nil }
        }
        .sheet(isPresented: $isAddingRule) {
            RouteRuleForm(rule: newRule, ru: ru) { changed in
                rules.append(changed)
                isAddingRule = false
            } onCancel: { isAddingRule = false }
        }
        .confirmationDialog(
            tr("Заменить черновик правил?", "Replace the routing draft?"),
            isPresented: $importConfirmation,
            titleVisibility: .visible
        ) {
            Button(tr("Импортировать и обновить", "Import and refresh")) { performImport() }
            Button(tr("Отмена", "Cancel"), role: .cancel) { importPath = nil }
        } message: {
            Text(tr("Импорт сохраняет правила в базе. Текущий черновик будет заменён сохранёнными правилами после импорта.", "Import saves rules to the database. The current draft will be replaced with the saved rules after import."))
        }
        .alert(tr("Не удалось импортировать", "Import failed"), isPresented: Binding(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )) {
            Button(tr("OK", "OK"), role: .cancel) { errorMessage = nil }
        } message: {
            Text(errorMessage ?? "")
        }
    }

    private var newRule: RouteRuleDraft {
        var rule = RouteRuleDraft()
        rule.conditions = [RouteConditionDraft()]
        return rule
    }

    @ViewBuilder private func ruleRow(_ rule: RouteRuleDraft, at index: Int) -> some View {
        HStack(spacing: 10) {
            Toggle(tr("Правило включено", "Rule enabled"), isOn: ruleEnabledBinding(at: index))
                .labelsHidden()
                .accessibilityLabel(tr("Включить правило", "Enable rule"))

            Button { editingRule = rule } label: {
                VStack(alignment: .leading, spacing: 4) {
                    Text(rule.name.isEmpty ? summary(rule) : rule.name)
                        .font(.headline)
                        .foregroundStyle(.primary)
                    Text("\(rule.matchMode.title(ru: ru)) · \(rule.action.title(ru: ru)) · \(rule.conditions.count) \(tr("условий", "conditions"))")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityHint(tr("Открыть редактор правила", "Open rule editor"))

            VStack(spacing: 2) {
                Button { moveRule(from: index, by: -1) } label: { Image(systemName: "chevron.up") }
                    .disabled(index == 0)
                    .help(tr("Выше по приоритету", "Move higher"))
                    .accessibilityLabel(tr("Выше по приоритету", "Move higher"))
                Button { moveRule(from: index, by: 1) } label: { Image(systemName: "chevron.down") }
                    .disabled(index == rules.count - 1)
                    .help(tr("Ниже по приоритету", "Move lower"))
                    .accessibilityLabel(tr("Ниже по приоритету", "Move lower"))
            }
            .buttonStyle(.borderless)

            Button(role: .destructive) { rules.remove(at: index) } label: { Image(systemName: "trash") }
                .buttonStyle(.borderless)
                .help(tr("Удалить правило", "Delete rule"))
                .accessibilityLabel(tr("Удалить правило", "Delete rule"))
        }
        .padding(.vertical, 5)
        .opacity(rule.enabled ? 1 : 0.58)
        .contextMenu {
            Button(tr("Изменить", "Edit")) { editingRule = rule }
            Button(tr("Выше по приоритету", "Move higher")) { moveRule(from: index, by: -1) }.disabled(index == 0)
            Button(tr("Ниже по приоритету", "Move lower")) { moveRule(from: index, by: 1) }.disabled(index == rules.count - 1)
            Button(tr("Удалить", "Delete"), role: .destructive) { rules.remove(at: index) }
        }
    }

    private func ruleEnabledBinding(at index: Int) -> Binding<Bool> {
        Binding(
            get: { rules.indices.contains(index) ? rules[index].enabled : false },
            set: { if rules.indices.contains(index) { rules[index].enabled = $0 } }
        )
    }

    private func moveRule(from index: Int, by offset: Int) {
        let destination = index + offset
        guard rules.indices.contains(index), rules.indices.contains(destination) else { return }
        rules.swapAt(index, destination)
    }

    private func summary(_ rule: RouteRuleDraft) -> String {
        guard let condition = rule.conditions.first, let value = condition.values.first, !value.isEmpty else {
            return tr("Новое правило", "New rule")
        }
        return "\(condition.kind.title(ru: ru)): \(value)"
    }

    private func chooseImportFile() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.database, .data]
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.message = tr("Выберите старую базу данных, из которой нужно перенести правила.", "Choose the legacy database to import routing rules from.")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        importPath = url.path
        importConfirmation = true
    }

    private func performImport() {
        guard let path = importPath else { return }
        importing = true
        Task { @MainActor in
            do {
                _ = try await model.perform("import_routes", ["path": path])
                let refreshed = try RouteRuleDraft.decode(model.settings["route_rules"])
                rules = refreshed
                onImported(RouteRuleDraft.encode(refreshed))
                importPath = nil
            } catch {
                errorMessage = error.localizedDescription
            }
            importing = false
        }
    }
}

private struct RouteRuleForm: View {
    @Environment(\.dismiss) private var dismiss
    @State private var rule: RouteRuleDraft
    @State private var validationError: String?
    let ru: Bool
    let onSave: (RouteRuleDraft) -> Void
    let onCancel: () -> Void

    init(rule: RouteRuleDraft, ru: Bool, onSave: @escaping (RouteRuleDraft) -> Void, onCancel: @escaping () -> Void) {
        _rule = State(initialValue: rule)
        self.ru = ru
        self.onSave = onSave
        self.onCancel = onCancel
    }

    private func tr(_ ruText: String, _ enText: String) -> String { ru ? ruText : enText }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    VStack(alignment: .leading, spacing: 14) {
                        sectionTitle(tr("Основное", "Rule"))
                        VStack(alignment: .leading, spacing: 6) {
                            Text(tr("Название (необязательно)", "Name (optional)")).font(.callout.weight(.medium))
                            TextField(tr("Например: локальные сайты", "For example: local sites"), text: $rule.name)
                                .textFieldStyle(.roundedBorder)
                        }
                        Toggle(tr("Включено", "Enabled"), isOn: $rule.enabled)
                        labeledPicker(tr("Действие", "Action"), selection: $rule.action) {
                            ForEach(RouteActionChoice.allCases) { action in Text(action.title(ru: ru)).tag(action) }
                        }
                        if rule.conditions.count > 1 {
                            labeledPicker(tr("Объединение условий", "Match mode"), selection: $rule.matchMode) {
                                ForEach(RouteMatchMode.allCases) { mode in Text(mode.title(ru: ru)).tag(mode) }
                            }
                            Text(tr("«Любое» подходит для списков исключений; «все» требует совпадения каждого условия.", "Use “any” for exception lists; “all” requires every condition to match."))
                                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .padding(16)
                    .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 12))

                    VStack(alignment: .leading, spacing: 12) {
                        sectionTitle(tr("Условия", "Conditions"))
                        Text(tr("Введите по одному значению в строке.", "Enter one value per line."))
                            .font(.caption).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                        ForEach($rule.conditions) { $condition in
                            ConditionForm(condition: $condition, ru: ru) {
                                rule.conditions.removeAll { $0.id == condition.id }
                            }
                        }
                        Button { rule.conditions.append(RouteConditionDraft()) } label: {
                            Label(tr("Добавить условие", "Add condition"), systemImage: "plus")
                        }
                        .accessibilityHint(tr("Добавить ещё одно условие", "Add another condition"))
                    }
                }
                .frame(maxWidth: 680, alignment: .leading)
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .top)
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                HStack {
                    Button(tr("Отмена", "Cancel"), action: onCancel)
                        .keyboardShortcut(.cancelAction)
                    Spacer()
                    Button(tr("Сохранить", "Save"), action: save)
                        .keyboardShortcut(.defaultAction)
                        .buttonStyle(.borderedProminent)
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 12)
                .background(.bar)
            }
            .navigationTitle(tr("Правило маршрутизации", "Routing rule"))
            .alert(tr("Проверьте правило", "Check the rule"), isPresented: Binding(
                get: { validationError != nil }, set: { if !$0 { validationError = nil } }
            )) {
                Button(tr("OK", "OK"), role: .cancel) { validationError = nil }
            } message: { Text(validationError ?? "") }
        }
        .frame(minWidth: 540, idealWidth: 620, minHeight: 560)
    }

    private func sectionTitle(_ title: String) -> some View {
        Text(title).font(.headline)
    }

    private func labeledPicker<Selection: Hashable, Content: View>(
        _ title: String,
        selection: Binding<Selection>,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.callout.weight(.medium))
            Picker(title, selection: selection, content: content)
                .labelsHidden()
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func save() {
        rule.conditions = rule.conditions.map { condition in
            var condition = condition
            condition.values = condition.values.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
            return condition
        }.filter { !$0.values.isEmpty }
        guard !rule.conditions.isEmpty else {
            validationError = tr("Добавьте хотя бы одно непустое условие.", "Add at least one condition with a value.")
            return
        }
        onSave(rule)
    }
}

private struct ConditionForm: View {
    @Binding var condition: RouteConditionDraft
    let ru: Bool
    let remove: () -> Void

    private func tr(_ ruText: String, _ enText: String) -> String { ru ? ruText : enText }
    private var valuesText: Binding<String> {
        Binding(
            get: { condition.values.joined(separator: "\n") },
            set: { condition.values = $0.components(separatedBy: .newlines) }
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .center) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(tr("Тип условия", "Condition type")).font(.callout.weight(.medium))
                    Picker(tr("Тип условия", "Condition type"), selection: $condition.kind) {
                        ForEach(RouteConditionKind.allCases) { kind in Text(kind.title(ru: ru)).tag(kind) }
                    }
                    .labelsHidden()
                }
                Spacer(minLength: 8)
                Button(role: .destructive, action: remove) {
                    Label(tr("Убрать", "Remove"), systemImage: "trash")
                }
                .buttonStyle(.borderless)
                .help(tr("Убрать условие", "Remove condition"))
                .accessibilityLabel(tr("Убрать условие", "Remove condition"))
            }
            Text(tr("Значения", "Values")).font(.callout.weight(.medium))
            TextEditor(text: valuesText)
                .font(.system(.body, design: .monospaced))
                .frame(height: 96)
                .scrollContentBackground(.hidden)
                .padding(6)
                .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.secondary.opacity(0.2)))
                .accessibilityLabel(tr("Значения условия", "Condition values"))
            Text(tr("Пример: ", "Example: ") + condition.kind.hint)
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 10))
    }
}

private struct ContentUnavailableViewCompat: View {
    let title: String
    let detail: String
    let symbol: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: symbol).font(.system(size: 28)).foregroundStyle(.secondary)
            Text(title).font(.headline)
            Text(detail).font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 24)
    }
}
