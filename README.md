# CompEditor

CompEditor ist ein lokaler Docker-Compose-Editor für Linux, macOS und Windows. Die Projektleiste zeigt die Services, Netzwerke und Volumes einer YAML-Datei. Ziehe ihren rechten Rand, um die Breite zu ändern. Die Listen für Services, Netzwerke und Volumes lassen sich durchsuchen. Dort kannst du auch neue Einträge anlegen. In der Mitte sind Konfiguration, Verknüpfungen, weitere Compose-Parameter und verfügbare Docker-Hub-Infos über Reiter erreichbar. Die Sprache der Oberfläche lässt sich in den Einstellungen dauerhaft zwischen Deutsch und Englisch wechseln.

## Starten

Das Linux-AppImage ausführbar machen und starten. Auf Systemen ohne FUSE kann es mit `--appimage-extract-and-run` gestartet werden. Der Linux-Dateidialog verwendet das Desktop-Portal mit Zenity als Ausweichlösung.

```sh
chmod +x CompEditor-0.1.0-x86_64.AppImage
./CompEditor-0.1.0-x86_64.AppImage
```

`examples/compose.yaml` eignet sich zum Ausprobieren. Die Datei kann auch als Startargument übergeben werden. Formulare übernehmen Änderungen mit **Übernehmen** in das geöffnete Dokument. Im Menü **Datei** schreibt **Speichern** die Datei und **Speichern unter** wählt einen neuen Speicherort. Das Menü **Bearbeiten** bietet die Standardbefehle für das aktive Eingabefeld. CompEditor hält lokale Änderungen bei einem Speicherkonflikt im Arbeitsspeicher. **Datei schließen** führt zur Startansicht zurück und fragt bei ungespeicherten Änderungen nach.

Über **Neues Projekt** lässt sich in einem ausgewählten leeren Ordner ein neues Projekt anlegen. CompEditor erstellt dort `docker-compose.yml`, eine leere `.env` und die initiale Historie. Vorher wird geprüft, dass keine dieser drei Dateien bereits existiert; CompEditor überschreibt sie nicht.

In der Projektleiste lassen sich über **Service hinzufügen** weitere Services anlegen. Der Name ist erforderlich, ein Image optional. Der neue Service wird direkt geöffnet und erscheint als Änderung in der Historie.

Die Service-Details enthalten eine Dropdown-Auswahl für die Neustart-Richtlinie (`no`, `always`, `on-failure` und `unless-stopped`), einschließlich gängiger Retry-Limits für `on-failure`. Ein nicht gesetzter Wert lässt die Compose-Standardrichtlinie gelten.

Unter **Image-Info** erkennt CompEditor tabellarisch dokumentierte Umgebungsparameter aus der Docker-Hub-Beschreibung und öffnet sie über **Für Service konfigurieren** im Environment-Formular. Pflicht- oder optionale Angaben werden nur übernommen, wenn sie dort ausdrücklich markiert sind; sonst steht der Status auf **Nicht angegeben**. Docker-Hub-Beschreibungen sind Freitext und enthalten nicht immer eine solche Tabelle.

Die Projektleiste zeigt die lokal deklarierten Services. Externe `include`- und `extends`-Dateien werden in dieser Version nicht geladen. Geteilte oder mehrdeutige YAML-Werte sind schreibgeschützt. Die App berücksichtigt den YAML-Aufbau. Eine vollständige Compose-Validierung ist separat mit `docker compose config --quiet` möglich.

## Variablen aus `.env`

CompEditor lädt die `.env` neben der geöffneten Compose-Datei. In breiten Fenstern stehen die Variablen und ihre Werte standardmäßig rechts neben den Service-Details. Ziehe den linken Rand der Seitenleiste, um ihre Breite zu ändern. Lange Variablennamen werden umgebrochen. Gelb markiert Variablen, die in der geöffneten Compose-Datei weder direkt noch über einen anderen .env-Wert verwendet werden. Die Schaltfläche **.env** blendet den Bereich aus oder ein. In kleineren Fenstern öffnet sie die Werte vorübergehend anstelle des Editors. Ein Klick auf einen Namen oder Wert aktiviert die Bearbeitung direkt in der Wertzelle. In den Service-Details sind .env-Verweise direkt im jeweiligen Eintrag verlinkt. Fehlende oder nicht aufgelöste Variablen erscheinen rot. Ein Klick springt zur passenden Zeile in der .env-Ansicht. **In .env speichern** aktualisiert die Vorschau. Bei einer fehlenden Datei legt dieser Knopf die `.env` an. Bearbeitete Werte werden wörtlich gespeichert.

Die Vorschau verwendet die `.env` und Standardwerte aus den Compose-Ausdrücken. Beim Speichern der YAML-Datei bleiben die Platzhalter erhalten.

Für Docker-Hub-Images zeigt die Service-Detailansicht die Beschreibung, Pull-Zahl, Sterne und das letzte Aktualisierungsdatum an. Aufgelöste Image-Platzhalter aus `.env` werden ebenfalls berücksichtigt. Images anderer Registries werden nicht abgefragt; die Metadaten benötigen eine Internetverbindung.

Im Image-Formular lässt sich für öffentliche Docker-Hub-Repositories ein Tag aus einer nach Aktualisierungsdatum sortierten Liste wählen. Die Auswahl ändert nur den Tag im Image-Namen; alle übrigen Compose-Einträge bleiben erhalten.

## Historie

Die Schaltfläche **Historie** öffnet die Änderungsliste mit dem neuesten Stand oben. YAML-Änderungen erscheinen zunächst als Entwürfe und werden beim Speichern in die Historie übernommen. Änderungen an der `.env` werden beim Speichern des Werts festgehalten.

**Wiederherstellen** setzt die YAML-Datei und die `.env` sofort auf den gewählten gespeicherten Stand. Damit werden alle dazwischenliegenden Änderungen gemeinsam rückgängig gemacht. CompEditor behält den geöffneten Service bei und springt rechts zur betroffenen Variable. Spätere Stände sind bis zur nächsten Änderung oder bis zum Schließen der Datei noch anwählbar. Beim erneuten Öffnen endet die Historie am zuletzt gewählten Stand. Beim Sprung zu einem Entwurf bleiben die Dateien auf der Festplatte bis zum nächsten Speichern unverändert.

CompEditor speichert die Historie pro Compose-Datei. Zu `compose.yaml` gehört `.compose.yaml.compeditor-history.json` im selben Ordner. Die Verlaufsdatei enthält frühere YAML- und `.env`-Inhalte einschließlich ihrer Werte.

## Bauen und testen

```sh
cargo run --locked -- examples/compose.yaml
cargo test --locked
cargo build --release --locked
```

Unter Linux erstellt `scripts/package-appimage.sh` das AppImage in `dist/`. Für breite Linux-Kompatibilität wird das Binärprogramm im Workflow unter Ubuntu 22.04 gebaut. Unter macOS und Windows erzeugt `cargo-packager` die nativen Pakete. Der Workflow **Build CompEditor** stellt sie als Artefakte für Apple Silicon, Intel-Macs und Windows x86_64 bereit. Die Artefakte werden ohne Herausgebersignatur erstellt.
