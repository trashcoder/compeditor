# Docksie

Docksie ist ein lokaler Docker-Compose-Editor für Linux, macOS und Windows. Er zeigt die Services einer YAML-Datei als Karten und bietet Formulare für Images, Ports und Umgebungsvariablen. Die Sprache der Oberfläche lässt sich in den Einstellungen dauerhaft zwischen Deutsch und Englisch wechseln.

## Starten

Das Linux-AppImage ausführbar machen und starten. Auf Systemen ohne FUSE kann es mit `--appimage-extract-and-run` gestartet werden. Der Linux-Dateidialog verwendet das Desktop-Portal mit Zenity als Ausweichlösung.

```sh
chmod +x Docksie-0.1.0-x86_64.AppImage
./Docksie-0.1.0-x86_64.AppImage
```

`examples/compose.yaml` eignet sich zum Ausprobieren. Die Datei kann auch als Startargument übergeben werden. Formulare übernehmen Änderungen mit **Übernehmen** in das geöffnete Dokument. **Speichern** schreibt die Datei, **Speichern unter** wählt einen neuen Speicherort. Docksie hält lokale Änderungen bei einem Speicherkonflikt im Arbeitsspeicher. **Datei schließen** führt zur Startansicht zurück und fragt bei ungespeicherten Änderungen nach.

Über **Neu** lässt sich in einem ausgewählten leeren Ordner ein neues Projekt anlegen. Docksie erstellt dort `docker-compose.yml`, eine leere `.env` und die initiale Historie. Vorher wird geprüft, dass keine dieser drei Dateien bereits existiert; Docksie überschreibt sie nicht.

In der Service-Übersicht lassen sich über **Service hinzufügen** weitere Services anlegen. Der Name ist erforderlich, ein Image optional. Der neue Service wird direkt geöffnet und erscheint als Änderung in der Historie.

Die Service-Details enthalten eine Dropdown-Auswahl für die Neustart-Richtlinie (`no`, `always`, `on-failure` und `unless-stopped`), einschließlich gängiger Retry-Limits für `on-failure`. Ein nicht gesetzter Wert lässt die Compose-Standardrichtlinie gelten.

In den Service-Details erkennt Docksie tabellarisch dokumentierte Umgebungsparameter aus der Docker-Hub-Beschreibung und öffnet sie über **Für Service konfigurieren** im Environment-Formular. Pflicht- oder optionale Angaben werden nur übernommen, wenn sie dort ausdrücklich markiert sind; sonst steht der Status auf **Nicht angegeben**. Docker-Hub-Beschreibungen sind Freitext und enthalten nicht immer eine solche Tabelle.

Die Karten zeigen die lokal deklarierten Services. Externe `include`- und `extends`-Dateien werden in dieser Version nicht geladen. Geteilte oder mehrdeutige YAML-Werte sind schreibgeschützt. Die App berücksichtigt den YAML-Aufbau. Eine vollständige Compose-Validierung ist separat mit `docker compose config --quiet` möglich.

## Variablen aus `.env`

Docksie lädt die `.env` neben der geöffneten Compose-Datei und zeigt rechts die Variablennamen und ihre Werte an. Ein Klick auf einen Namen oder Wert öffnet das Eingabefeld direkt in der Zeile. Die Variablenlinks in den Service-Details springen zur passenden Zeile. **In .env speichern** aktualisiert die Vorschau. Bei einer fehlenden Datei legt dieser Knopf die `.env` an. Bearbeitete Werte werden wörtlich gespeichert.

Die Vorschau verwendet die `.env` und Standardwerte aus den Compose-Ausdrücken. Beim Speichern der YAML-Datei bleiben die Platzhalter erhalten.

Für Docker-Hub-Images zeigt die Service-Detailansicht die Beschreibung, Pull-Zahl, Sterne und das letzte Aktualisierungsdatum an. Aufgelöste Image-Platzhalter aus `.env` werden ebenfalls berücksichtigt. Images anderer Registries werden nicht abgefragt; die Metadaten benötigen eine Internetverbindung.

Im Image-Formular lässt sich für öffentliche Docker-Hub-Repositories ein Tag aus einer nach Aktualisierungsdatum sortierten Liste wählen. Die Auswahl ändert nur den Tag im Image-Namen; alle übrigen Compose-Einträge bleiben erhalten.

## Historie

Die rechte Spalte zeigt Änderungen chronologisch mit dem neuesten Stand oben. YAML-Änderungen erscheinen zunächst als Entwürfe und werden beim Speichern in die Historie übernommen. Änderungen an der `.env` werden beim Speichern des Werts festgehalten.

**Wiederherstellen** setzt die YAML-Datei und die `.env` sofort auf den gewählten gespeicherten Stand. Damit werden alle dazwischenliegenden Änderungen gemeinsam rückgängig gemacht. Docksie behält den geöffneten Service bei und springt rechts zur betroffenen Variable. Spätere Stände sind bis zur nächsten Änderung oder bis zum Schließen der Datei noch anwählbar. Beim erneuten Öffnen endet die Historie am zuletzt gewählten Stand. Beim Sprung zu einem Entwurf bleiben die Dateien auf der Festplatte bis zum nächsten Speichern unverändert.

Docksie speichert die Historie pro Compose-Datei. Zu `compose.yaml` gehört `.compose.yaml.docksie-history.json` im selben Ordner. Die Datei enthält frühere YAML- und `.env`-Inhalte einschließlich ihrer Werte.

## Bauen und testen

```sh
cargo run --locked -- examples/compose.yaml
cargo test --locked
cargo build --release --locked
```

Unter Linux erstellt `scripts/package-appimage.sh` das AppImage in `dist/`. Für breite Linux-Kompatibilität wird das Binärprogramm im Workflow unter Ubuntu 22.04 gebaut. Unter macOS und Windows erzeugt `cargo-packager` die nativen Pakete. Der Workflow **Build Docksie** stellt sie als Artefakte für Apple Silicon, Intel-Macs und Windows x86_64 bereit. Die Artefakte werden ohne Herausgebersignatur erstellt.
