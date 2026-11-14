import SwiftUI
import AppKit
import UniformTypeIdentifiers

@MainActor private func throneIcon() -> NSImage {
    if let url=throneResource("AppIcon",extension:"png"),let image=NSImage(contentsOf:url){return image}
    NSLog("Throne: missing Resources/AppIcon.png; application logo unavailable")
    return NSImage(size:NSSize(width:32,height:32))
}

@main struct ThroneApp: App {
    @StateObject private var model = Model()
    @NSApplicationDelegateAdaptor(AppLifecycle.self) private var lifecycle
    var body: some Scene {
        Window("Throne GTK", id: "main") { MainView().environmentObject(model).frame(minWidth: 900, minHeight: 620).background(MainWindowAccessor(lifecycle: lifecycle)).task { lifecycle.model = model; model.start() } }
            .commands {
                CommandGroup(replacing: .newItem) {}
                CommandGroup(replacing: .appTermination) {
                    Button(model.tr("Выйти из Throne GTK", "Quit Throne GTK")) { NSApp.terminate(nil) }.keyboardShortcut("q")
                }
                CommandGroup(replacing:.appInfo){Button(model.tr("О программе Throne GTK","About Throne GTK")){let a=NSAlert();a.messageText="Throne GTK";a.informativeText=model.tr("Нативный клиент прокси и VPN.","Native proxy and VPN client.");a.runModal()}}
            }
        Settings { SettingsView().environmentObject(model).frame(width: 650, height: 650) }
    }
}

struct MainView: View {
    @EnvironmentObject var m: Model
    @State private var search = ""
    @State private var groupID = 0
    @State private var appliedStartupVisibility = false
    @State private var confirmDeleteGroup = false
    @State private var pendingProfileDelete: [String: Any]?
    var body: some View {
        NavigationSplitView {
            VStack(spacing: 0) {
                HStack { Image(nsImage:throneIcon()).resizable().frame(width:32,height:32); VStack(alignment:.leading){Text("Throne GTK").font(.system(.headline, design:.rounded).weight(.black));Text(m.tr("ПОДКЛЮЧЕНИЕ","CONNECTION")).font(.caption2.monospaced()).foregroundStyle(.secondary)} }.padding(20).frame(maxWidth:.infinity,alignment:.leading)
                VStack(spacing:3) { nav("profiles", "server.rack", m.tr("Профили","Profiles")); nav("connections", "point.3.connected.trianglepath.dotted", m.tr("Соединения","Connections")); nav("logs", "text.alignleft", m.tr("Журнал","Logs")); nav("settings", "gearshape", m.tr("Настройки","Settings")) }.padding(.horizontal,10)
                Spacer()
                VStack(alignment:.leading,spacing:8){ Label(statusTitle, systemImage:statusIcon).font(.callout.weight(.semibold)); Text(m.tr("Локальный прокси", "Local proxy") + " · " + (m.settings["mixed_port"].map { String(describing:$0) } ?? "2080")).font(.caption).foregroundStyle(.secondary) }.frame(maxWidth:.infinity,alignment:.leading).padding(16).background(.quaternary.opacity(0.45),in:RoundedRectangle(cornerRadius:14)).padding(12)
            }.navigationSplitViewColumnWidth(min:210, ideal:240)
        } detail: {
            VStack(spacing:0) { header; if let notice=m.operationNotice ?? (m.snapshot["notice"] as? String),!notice.isEmpty {Label(notice,systemImage:"info.circle").font(.caption).foregroundStyle(.secondary).frame(maxWidth:.infinity,alignment:.leading).padding(.horizontal,28).padding(.vertical,8)}; if !m.ready {ProgressView(m.tr("Загрузка данных Throne GTK…","Loading Throne GTK data…")).frame(maxWidth:.infinity,maxHeight:.infinity)} else {Group { switch m.tab { case "connections": ConnectionsView(); case "logs": LogsView(); case "settings": SettingsView(); default: profileList } }.frame(maxWidth:.infinity,maxHeight:.infinity)} }
        }.onChange(of:m.ready){ready in if ready { reconcileGroupSelection() };if ready,!appliedStartupVisibility {appliedStartupVisibility=true;if m.settings["start_minimized"] as? Bool == true {NSApp.windows.first(where: { $0.identifier?.rawValue == "throne-main" })?.orderOut(nil)}}}.onChange(of: groupIDs) { _ in if m.ready { reconcileGroupSelection() } }.confirmationDialog(m.tr("Удалить группу?","Delete this group?"),isPresented:$confirmDeleteGroup,titleVisibility:.visible){Button(m.tr("Удалить","Delete"),role:.destructive){m.command("group_delete",["id":groupID])};Button(m.tr("Отмена","Cancel"),role:.cancel){}}.confirmationDialog(m.tr("Удалить профиль?","Delete profile?"),isPresented:Binding(get:{pendingProfileDelete != nil},set:{if !$0 {pendingProfileDelete=nil}}),titleVisibility:.visible){Button(m.tr("Удалить","Delete"),role:.destructive){if let profile=pendingProfileDelete {m.command("profile_delete",["ids":[profile["id"] ?? 0]])};pendingProfileDelete=nil};Button(m.tr("Отмена","Cancel"),role:.cancel){}}.alert(m.tr("Ошибка","Something went wrong"), isPresented: Binding(get:{m.error != nil},set:{if !$0 {m.error=nil}})) { Button(m.tr("OK","OK"),role:.cancel){m.error=nil} } message: { Text(m.error ?? "") }.alert(m.tr("Подключить VPN?","Connect VPN?"),isPresented:$m.confirmStartupVPN){Button(m.tr("Продолжить","Continue")){m.command("connect",["acknowledge_vpn":true])};Button(m.tr("Не сейчас","Not now"),role:.cancel){}} message:{Text(m.tr("macOS может запросить разрешение на VPN и открыть системные настройки.","macOS may request VPN approval and open System Settings."))}
    }
    private var statusTitle: String { switch statusKind { case "connected": return m.tr("Подключено","Connected"); case "connecting":return m.tr("Подключение…","Connecting…"); case "failed":return m.tr("Ошибка подключения","Connection failed"); default:return m.tr("Отключено","Disconnected") } }
    private func nav(_ id:String,_ icon:String,_ label:String)->some View { Button { m.tab=id } label:{Label(label,systemImage:icon).frame(maxWidth:.infinity,alignment:.leading).padding(9).contentShape(RoundedRectangle(cornerRadius:8)).background(m.tab == id ? Color.accentColor.opacity(0.13):.clear,in:RoundedRectangle(cornerRadius:8))}.buttonStyle(.plain).accessibilityLabel(label) }
    private var statusKind:String { m.status["kind"] as? String ?? "disconnected" }
    private var statusIcon:String { statusKind == "connected" ? "checkmark.circle.fill" : statusKind == "connecting" ? "arrow.triangle.2.circlepath" : statusKind == "failed" ? "exclamationmark.circle.fill" : "circle" }
    private var header: some View { HStack { VStack(alignment:.leading,spacing:3){Text(title).font(.system(size:26,weight:.bold,design:.rounded));Text(subtitle).font(.subheadline).foregroundStyle(.secondary);if let stage=m.snapshot["speed_stage"] as? String,!stage.isEmpty {Text(stage).font(.caption).foregroundStyle(.secondary)} else if m.snapshot["testing"] as? Bool == true {Text(m.tr("Проверка задержки…","Testing latency…")).font(.caption).foregroundStyle(.secondary)}}; Spacer(); if m.tab == "profiles" { Button { chooseImport() } label:{Label(m.tr("Импорт","Import"),systemImage:"square.and.arrow.down")}; Button { addProfile() } label:{Label(m.tr("Добавить","Add"),systemImage:"plus")};Button {if m.snapshot["testing"] as? Bool == true {m.command("stop_test")}else{m.command("test_latency",["ids":[],"gid":groupID])}} label:{Image(systemName:(m.snapshot["testing"] as? Bool ?? false) ? "stop.fill":"waveform.path.ecg")}.help((m.snapshot["testing"] as? Bool ?? false) ? m.tr("Остановить проверку","Stop latency tests"):m.tr("Проверить задержку","Test latency")).accessibilityLabel((m.snapshot["testing"] as? Bool ?? false) ? m.tr("Остановить проверку","Stop latency tests"):m.tr("Проверить задержку","Test latency")).disabled(m.busy) }; if statusKind == "connected" || statusKind == "connecting" { Button(m.tr("Отключить","Disconnect")){m.command("disconnect")}.buttonStyle(.bordered) } else if m.tab == "profiles" { Button(m.tr("Подключить","Connect")){connect()}.buttonStyle(.borderedProminent).keyboardShortcut(.return,modifiers:[]).disabled(m.busy || m.shutdownPending) } }.padding(.horizontal,28).padding(.vertical,20).background(.bar) }
    private var profileSelection: Binding<Int?> { Binding(get: { let id=m.snapshot["selected_id"] as? Int ?? 0;return id == 0 ? nil : id }, set: { if let id=$0 {m.command("select",["id":id])} }) }
    private var groupIDs: [Int] { m.groups.compactMap { $0["id"] as? Int } }
    private func reconcileGroupSelection() {
        groupID = ProfileSearch.reconciledGroupID(currentGroupID: groupID, groups: m.groups, profiles: m.profiles, selectedID: m.snapshot["selected_id"] as? Int ?? 0)
    }
    private var title:String { switch m.tab {case "profiles":return m.tr("Профили","Profiles");case "connections":return m.tr("Соединения","Connections");case "logs":return m.tr("Журнал","Logs");default:return m.tr("Настройки","Settings")} }
    private var subtitle:String { m.tr("Управляйте серверами и защищённым подключением","Manage your servers and secure connection") }
    private var profileList:some View { VStack(spacing:0){ HStack{Image(systemName:"magnifyingglass").foregroundStyle(.secondary);TextField(m.tr("Поиск профилей","Search profiles"),text:$search).textFieldStyle(.plain); Picker(m.tr("Группа","Group"),selection:$groupID){ForEach(Array(m.groups.enumerated()),id:\.offset){_,g in Text(g["name"] as? String ?? m.tr("Группа","Group")).tag(g["id"] as? Int ?? 0)}}.frame(maxWidth:200); if let group=m.groups.first(where:{($0["id"] as? Int)==groupID}), !((group["url"] as? String ?? "").isEmpty) { Button { m.command("subscription_refresh",["gid":groupID]) } label:{Image(systemName:"arrow.clockwise")}.help(m.tr("Обновить подписку","Refresh subscription")).accessibilityLabel(m.tr("Обновить подписку","Refresh subscription")) };Menu{Button(m.tr("Новая группа или подписка","New group or subscription")){manageGroups()};if m.groups.contains(where:{($0["id"] as? Int)==groupID}){Button(m.tr("Изменить группу","Edit group")){editGroup()};Button(m.tr("Удалить группу","Delete group"),role:.destructive){confirmDeleteGroup=true}}}label:{Image(systemName:"folder.badge.plus")}.help(m.tr("Группы и подписки","Groups and subscriptions")).accessibilityLabel(m.tr("Группы и подписки","Groups and subscriptions")) }.padding(12).background(.background,in:RoundedRectangle(cornerRadius:12)).padding(.horizontal,24).padding(.top,16)
        let filtered=m.profiles.filter{ProfileSearch.matches($0, groupID: groupID, query: search)}
        if filtered.isEmpty { EmptyState(title:m.tr("Профилей пока нет","No profiles yet"),icon:"network",detail:m.tr("Добавьте ссылку или импортируйте старую базу данных.","Add a server link or import your legacy database.")).frame(maxHeight:.infinity) } else { List(selection: profileSelection) { if filtered.count > 1 { Button { m.command("select",["id":-groupID]) } label:{HStack{Label(m.tr("Автовыбор","Automatic selection"),systemImage:"sparkles");Spacer();Text((m.snapshot["auto_status"] as? [String:Any])?["selected"] as? String ?? "").font(.caption).foregroundStyle(.secondary)}}.padding(12).buttonStyle(.plain).tag(-groupID).onTapGesture(count:2){connect(id: -groupID)}.contextMenu{Button(m.tr("Снять закрепление","Unpin automatic selection")){m.command("auto_pin",["name":""])};Button(m.tr("Перепроверить серверы","Retest servers")){m.command("auto_recheck")} } }; ForEach(Array(filtered.enumerated()),id:\.offset){ _,p in ProfileRow(profile:p, connected:(m.status["profile_id"] as? Int)==(p["id"] as? Int)).tag(p["id"] as? Int ?? 0).contentShape(Rectangle()).onTapGesture(count:2){connect(id: p["id"] as? Int)}.contextMenu { Button(m.tr("Изменить","Edit")){edit(p)}; Button(m.tr("Замерить скорость","Test speed")){m.command("speed_test",["id":p["id"] ?? 0])}; Button(m.tr("Проверить конфигурацию","Check configuration")){m.command("check_config",["id":p["id"] ?? 0])}; Button(m.tr("Закрепить в автовыборе","Pin for automatic selection")){m.command("auto_pin",["name":p["name"] ?? ""])}; Button(m.tr("Экспорт конфигурации","Export configuration")){export(p)}; Button(m.tr("Удалить","Delete"),role:.destructive){pendingProfileDelete=p} } } }.listStyle(.inset).scrollContentBackground(.hidden) }
        }.padding(.bottom,12) }
    private func chooseImport(){ let panel=NSOpenPanel();panel.allowedContentTypes=[.database,.json,.data];panel.allowsMultipleSelection=false;panel.canChooseDirectories=false; if panel.runModal() == .OK,let url=panel.url { m.command("import_throne",["path":url.path]) } }
    private func address(_ p:[String:Any])->String { let o=p["outbound"] as? [String:Any] ?? [:];return "\(o["server"] as? String ?? o["protocol"] as? String ?? "") \(o["server_port"] as? Int ?? 0)" }
    private func export(_ p:[String:Any]) { m.command("export_config",["id":p["id"] ?? 0]) { result in let data=(try? JSONSerialization.data(withJSONObject:result ?? NSNull(),options:[.prettyPrinted,.sortedKeys])) ?? Data(String(describing:result ?? "").utf8);let text=String(decoding:data,as:UTF8.self);let alert=NSAlert();alert.messageText=m.tr("Конфигурация","Configuration");let view=NSScrollView(frame:NSRect(x:0,y:0,width:520,height:360));let editor=NSTextView(frame:view.bounds);editor.isEditable=false;editor.string=text;editor.font = .monospacedSystemFont(ofSize:11,weight:.regular);view.documentView=editor;alert.accessoryView=view;alert.addButton(withTitle:m.tr("OK","OK"));alert.runModal() } }
    private func groupForm(name: NSTextField, url: NSTextField) -> NSView {
        let box = NSView(frame: NSRect(x: 0, y: 0, width: 430, height: 82))
        let nameLabel = NSTextField(labelWithString: m.tr("Название", "Name"))
        let urlLabel = NSTextField(labelWithString: m.tr("Адрес подписки", "Subscription URL"))
        name.setAccessibilityLabel(m.tr("Название группы", "Group name"))
        url.setAccessibilityLabel(m.tr("Адрес подписки, оставьте пустым для обычной группы", "Subscription URL; leave blank for a manual group"))
        name.frame = NSRect(x: 126, y: 48, width: 300, height: 24)
        nameLabel.frame = NSRect(x: 0, y: 48, width: 118, height: 24)
        url.frame = NSRect(x: 126, y: 10, width: 300, height: 24)
        urlLabel.frame = NSRect(x: 0, y: 10, width: 118, height: 24)
        box.addSubview(nameLabel)
        box.addSubview(name)
        box.addSubview(urlLabel)
        box.addSubview(url)
        return box
    }

    private func subscriptionHost(_ raw: String) -> String? {
        guard let components = URLComponents(string: raw),
              let scheme = components.scheme?.lowercased(),
              ["http", "https"].contains(scheme),
              let host = components.host, !host.isEmpty else { return nil }
        return host
    }

    private func manageGroups(){
        let alert=NSAlert();alert.messageText=m.tr("Новая группа или подписка","New group or subscription");alert.informativeText=m.tr("Оставьте адрес пустым для обычной группы. Укажите адрес HTTP(S), чтобы создать подписку.","Leave the URL blank for a manual group. Enter an HTTP(S) URL to create a subscription.")
        let name=NSTextField(string:"");name.placeholderString=m.tr("Название группы","Group name");let url=NSTextField(string:"");url.placeholderString=m.tr("https://…","https://…");alert.accessoryView=groupForm(name:name,url:url);alert.addButton(withTitle:m.tr("Создать","Create"));alert.addButton(withTitle:m.tr("Отмена","Cancel"))
        while alert.runModal() == .alertFirstButtonReturn {
            let urlText=url.stringValue.trimmingCharacters(in:.whitespacesAndNewlines)
            let groupName=name.stringValue.trimmingCharacters(in:.whitespacesAndNewlines)
            let resolvedName = groupName.isEmpty ? (urlText.isEmpty ? "" : subscriptionHost(urlText) ?? "") : groupName
            guard !resolvedName.isEmpty else { showGroupValidation(m.tr("Укажите название группы.","Enter a group name.")); continue }
            if !urlText.isEmpty && subscriptionHost(urlText) == nil { showGroupValidation(m.tr("Введите корректный адрес HTTP(S).","Enter a valid HTTP(S) subscription URL.")); continue }
            let group:[String:Any] = ["id":0,"name":resolvedName,"url":urlText,"info":"","archive":false,"skip_auto_update":false,"sub_last_update":0,"created_at":0,"updated_at":0]
            m.command("group_save",["group":group]){result in if let id=(result as? [String:Any])?["id"] as? Int {groupID=id;if !urlText.isEmpty {m.command("subscription_refresh",["gid":id])}}}
            return
        }
    }

    private func editGroup(){
        guard var group=m.groups.first(where:{($0["id"] as? Int)==groupID}) else { return }
        let alert=NSAlert();alert.messageText=m.tr("Изменить группу","Edit group");let name=NSTextField(string:group["name"] as? String ?? "");let url=NSTextField(string:group["url"] as? String ?? "");name.placeholderString=m.tr("Название группы","Group name");url.placeholderString=m.tr("https://…","https://…");alert.accessoryView=groupForm(name:name,url:url);alert.addButton(withTitle:m.tr("Сохранить","Save"));alert.addButton(withTitle:m.tr("Отмена","Cancel"))
        while alert.runModal() == .alertFirstButtonReturn {
            let urlText=url.stringValue.trimmingCharacters(in:.whitespacesAndNewlines)
            let groupName=name.stringValue.trimmingCharacters(in:.whitespacesAndNewlines)
            let resolvedName = groupName.isEmpty ? (urlText.isEmpty ? "" : subscriptionHost(urlText) ?? "") : groupName
            guard !resolvedName.isEmpty else { showGroupValidation(m.tr("Укажите название группы.","Enter a group name.")); continue }
            if !urlText.isEmpty && subscriptionHost(urlText) == nil { showGroupValidation(m.tr("Введите корректный адрес HTTP(S).","Enter a valid HTTP(S) subscription URL.")); continue }
            group["name"]=resolvedName;group["url"]=urlText
            m.command("group_save",["group":group]){_ in if !urlText.isEmpty {m.command("subscription_refresh",["gid":groupID])}}
            return
        }
    }

    private func showGroupValidation(_ message: String) {
        let warning = NSAlert()
        warning.messageText = m.tr("Проверьте данные группы", "Check the group details")
        warning.informativeText = message
        warning.addButton(withTitle: m.tr("Вернуться к форме", "Back to form"))
        warning.runModal()
    }
    private func connect(id: Int? = nil){
        var fields: [String:Any] = [:]
        if let id { fields["id"] = id }
        guard (m.settings["mode"] as? String) == "vpn" else { m.command("connect", fields); return }
        let alert=NSAlert();alert.messageText=m.tr("Подключить VPN?","Connect VPN?");alert.informativeText=m.tr("Для VPN macOS может запросить разрешение и открыть системные настройки. Продолжать?","macOS may ask for permission and open System Settings for VPN access. Continue?");alert.addButton(withTitle:m.tr("Продолжить","Continue"));alert.addButton(withTitle:m.tr("Отмена","Cancel"));if alert.runModal() == .alertFirstButtonReturn {fields["acknowledge_vpn"] = true;m.command("connect",fields)}
    }
    private func addProfile(){ let alert=NSAlert();alert.messageText=m.tr("Добавить профили","Add profiles");alert.informativeText=m.tr("Вставьте ссылки на серверы","Paste server links");let field=NSTextView(frame:NSRect(x:0,y:0,width:360,height:110));field.font = .monospacedSystemFont(ofSize:12,weight:.regular);alert.accessoryView=field;alert.addButton(withTitle:m.tr("Импортировать","Import"));alert.addButton(withTitle:m.tr("Отмена","Cancel"));if alert.runModal() == .alertFirstButtonReturn {m.command("import_links",["text":field.string,"gid":groupID])} }
    private func edit(_ p:[String:Any]) { ProfileEditor(profile:p,ru:m.language == "ru"){m.command("profile_save",["profile":$0])}.open() }
}

struct ProfileRow: View {
    @EnvironmentObject var m: Model
    let profile: [String: Any]
    let connected: Bool
    private var endpoint: String {
        let outbound = profile["outbound"] as? [String: Any] ?? [:]
        let host = outbound["server"] as? String ?? ""
        let port = outbound["server_port"] as? Int ?? 0
        return port > 0 ? "\(host):\(port)" : host
    }
    private var latency: String {
        let value = profile["latency"] as? Int ?? 0
        return value > 0 ? "\(value) \(m.tr("мс","ms"))" : value < 0 ? m.tr("Таймаут","Timeout") : "—"
    }
    private var speedSummary: String {
        let down = profile["dl_speed"] as? String ?? ""
        let up = profile["ul_speed"] as? String ?? ""
        let country = profile["test_country"] as? String ?? ""
        return [(down.isEmpty ? nil : "↓ \(down)"), (up.isEmpty ? nil : "↑ \(up)"), (country.isEmpty ? nil : country)].compactMap { $0 }.joined(separator: "  ")
    }
    var body: some View {
        HStack(spacing: 14) {
            ZStack {
                RoundedRectangle(cornerRadius: 11).fill(connected ? Color.accentColor.opacity(0.14) : Color.secondary.opacity(0.08))
                Image(systemName: connected ? "antenna.radiowaves.left.and.right" : "server.rack").foregroundStyle(connected ? Color.accentColor : Color.secondary)
            }.frame(width: 44, height: 44)
            VStack(alignment: .leading, spacing: 5) {
                Text(profile["name"] as? String ?? m.tr("Сервер","Server")).font(.headline)
                Text("\(profile["kind"] as? String ?? "proxy")  ·  \(endpoint)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                if !speedSummary.isEmpty { Text(speedSummary).font(.caption2.monospaced()).foregroundStyle(.secondary) }
            }
            Spacer()
            Text(latency).font(.system(.callout, design: .rounded).monospacedDigit()).foregroundStyle(.secondary)
            if connected { Image(systemName: "checkmark.circle.fill").foregroundStyle(.green) }
        }.padding(12).background(.background, in: RoundedRectangle(cornerRadius: 14)).overlay(RoundedRectangle(cornerRadius: 14).stroke(connected ? Color.accentColor.opacity(0.45) : Color.secondary.opacity(0.12)))
    }
}

struct ConnectionsView:View { @EnvironmentObject var m:Model
    var body:some View { let connections=m.snapshot["connections"] as? [[String:Any]] ?? []; VStack(alignment:.leading,spacing:16){metricStrip; if connections.isEmpty { EmptyState(title:m.tr("Нет активных соединений","No active connections"),icon:"point.3.connected.trianglepath.dotted",detail:m.tr("Соединения появятся здесь после подключения.","Connections will appear here when traffic starts."))} else {ScrollView{VStack(spacing:8){ForEach(Array(connections.enumerated()),id:\.offset){_,row in let domain=row["domain"] as? String ?? "";let dest=row["dest"] as? String ?? "—";let meta=[row["process"] as? String ?? "",(row["network"] as? String ?? "").uppercased()].filter{ !$0.isEmpty }.joined(separator:" · ");HStack{VStack(alignment:.leading){Text(domain.isEmpty ? dest:domain).lineLimit(1);Text(meta).font(.caption).foregroundStyle(.secondary)}.frame(maxWidth:.infinity,alignment:.leading);Text("↓ \(ByteCountFormatter.string(fromByteCount:(row["download"] as? Int64) ?? 0,countStyle:.binary))  ↑ \(ByteCountFormatter.string(fromByteCount:(row["upload"] as? Int64) ?? 0,countStyle:.binary))").font(.system(.caption,design:.monospaced))}.padding(10).background(.quaternary.opacity(0.3),in:RoundedRectangle(cornerRadius:8))}}}.padding(.horizontal,24)};Spacer()}.padding(.top,24) }
    private var metricStrip:some View { HStack{metric(m.tr("Загрузка","Download"),m.snapshot["traffic"] as? [String:Any],"down", "arrow.down");metric(m.tr("Отдача","Upload"),m.snapshot["traffic"] as? [String:Any],"up","arrow.up");metric(m.tr("Всего","Total"),m.snapshot["traffic"] as? [String:Any],"total","arrow.down.to.line")}.padding(.horizontal,24) }
    private func metric(_ title:String,_ data:[String:Any]?,_ key:String,_ icon:String)->some View { let raw:Int64 = key == "total" ? Int64((data?["total_down"] as? Int) ?? 0) + Int64((data?["total_up"] as? Int) ?? 0) : Int64((data?[key] as? Int) ?? 0); let value=ByteCountFormatter.string(fromByteCount:raw,countStyle:.binary) + (key == "total" ? "" : m.tr(" /с"," /s")); return VStack(alignment:.leading,spacing:10){Label(title,systemImage:icon).font(.caption).foregroundStyle(.secondary);Text(value).font(.title2.weight(.semibold).monospacedDigit())}.frame(maxWidth:.infinity,alignment:.leading).padding(16).background(.quaternary.opacity(0.45),in:RoundedRectangle(cornerRadius:14)) }
}

struct LogsView:View { @EnvironmentObject var m:Model
    var body:some View { let logs=m.snapshot["logs"] as? [String] ?? []; VStack(alignment:.leading){HStack{Text(m.tr("Последние события","Recent events")).font(.headline);Spacer();Button(m.tr("Скопировать всё","Copy all")){let pasteboard=NSPasteboard.general;pasteboard.clearContents();pasteboard.setString(logs.joined(separator:"\n"),forType:.string)}.disabled(logs.isEmpty);Button(m.tr("Очистить","Clear")){m.command("logs_clear")}}.padding(.horizontal,24).padding(.top,18);if !logs.isEmpty { List(Array(logs.reversed().enumerated()),id:\.offset){_,line in Text(line).font(.system(.caption,design:.monospaced)).textSelection(.enabled).padding(.vertical,3)}.listStyle(.plain) } else {EmptyState(title:m.tr("Журнал пуст","No log entries"),icon:"text.alignleft",detail:m.tr("События подключения появятся здесь.","Connection events will appear here."))} } }
}

struct EmptyState:View { let title:String;let icon:String;let detail:String;var body:some View { VStack(spacing:12){Image(systemName:icon).font(.system(size:34,weight:.light)).foregroundStyle(.secondary);Text(title).font(.title3.weight(.semibold));Text(detail).font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center).frame(maxWidth:340)}.frame(maxWidth:.infinity,maxHeight:.infinity).padding(28) } }

@MainActor struct ProfileEditor {
    let profile:[String:Any]
    let ru:Bool
    let save:([String:Any])->Void

    private func tr(_ ruText: String, _ enText: String) -> String { ru ? ruText : enText }

    func open(){
        let alert=NSAlert()
        alert.messageText=tr("Профиль","Profile")
        alert.informativeText=tr("Измените имя и JSON outbound. Поля протокола сохраняются, если вы их не меняете.","Edit the name and outbound JSON. Protocol-specific fields are preserved unless changed.")

        let box=NSView(frame:NSRect(x:0,y:0,width:480,height:390))
        let name=NSTextField(string:profile["name"] as? String ?? "")
        name.frame=NSRect(x:112,y:356,width:360,height:24)
        name.placeholderString=tr("Название","Name")
        name.setAccessibilityLabel(tr("Название профиля","Profile name"))
        let nameLabel=NSTextField(labelWithString:tr("Название","Name"))
        nameLabel.frame=NSRect(x:0,y:356,width:104,height:24)
        box.addSubview(nameLabel)
        box.addSubview(name)

        let source=profile["outbound"] ?? [:]
        let data=(try? JSONSerialization.data(withJSONObject:source,options:[.prettyPrinted,.sortedKeys])) ?? Data("{}".utf8)
        let editor=NSTextView(frame:NSRect(x:0,y:0,width:470,height:320))
        editor.font = .monospacedSystemFont(ofSize:11,weight:.regular)
        editor.string=String(decoding:data,as:UTF8.self)
        editor.setAccessibilityLabel(tr("JSON outbound","Outbound JSON"))
        let scroll=NSScrollView(frame:NSRect(x:0,y:12,width:472,height:330))
        scroll.hasVerticalScroller=true
        scroll.documentView=editor
        box.addSubview(scroll)
        alert.accessoryView=box
        alert.addButton(withTitle:tr("Сохранить","Save"))
        alert.addButton(withTitle:tr("Отмена","Cancel"))

        while alert.runModal() == .alertFirstButtonReturn {
            guard let json=editor.string.data(using:.utf8),
                  let outbound=try? JSONSerialization.jsonObject(with:json) as? [String:Any] else {
                let error=NSAlert()
                error.messageText=tr("Неверный JSON","Invalid JSON")
                error.informativeText=tr("Outbound должен быть корректным JSON-объектом. Исправьте текст в редакторе.","Outbound must be a valid JSON object. Correct the text in the editor.")
                error.addButton(withTitle:tr("Вернуться к редактору","Return to editor"))
                error.runModal()
                continue
            }
            var value=profile
            value["name"]=name.stringValue
            value["kind"]=outbound["type"] as? String ?? profile["kind"]
            value["outbound"]=outbound
            save(value)
            return
        }
    }
}
