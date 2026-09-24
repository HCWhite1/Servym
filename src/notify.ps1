# Notification du système pour src/notify.js. Les textes arrivent par variables
# d'environnement et sont insérés comme nœuds texte : jamais interprétés.
$ErrorActionPreference = 'Stop'
[void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
[void][Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime]
$xml = [Windows.Data.Xml.Dom.XmlDocument]::new()
$xml.LoadXml('<toast><visual><binding template="ToastGeneric"><text/><text/><text placement="attribution"/></binding></visual></toast>')
$texts = $xml.GetElementsByTagName('text')
[void]$texts.Item(0).AppendChild($xml.CreateTextNode($env:SERVYNEX_NOTIFY_TITLE))
[void]$texts.Item(1).AppendChild($xml.CreateTextNode($env:SERVYNEX_NOTIFY_BODY))
[void]$texts.Item(2).AppendChild($xml.CreateTextNode($env:SERVYNEX_NOTIFY_SOURCE))
# Identifiant d'application de Windows PowerShell : déjà connu de Windows, sans raccourci à créer.
$app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($xml))
