// SPDX-License-Identifier: AGPL-3.0-or-later
//! Minimal translation table. Keys are short identifiers; unknown keys are shown as-is.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Fr];

    pub fn detect() -> Self {
        let locale = ["LC_ALL", "LANG", "LANGUAGE"]
            .iter()
            .find_map(|v| std::env::var(v).ok())
            .unwrap_or_default();
        if locale.starts_with("fr") || windows_ui_is_french() {
            Lang::Fr
        } else {
            Lang::En
        }
    }

    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Fr => "Français",
        }
    }

    pub fn t(self, key: &'static str) -> &'static str {
        let col = self as usize;
        TABLE
            .iter()
            .find(|row| row[0] == key)
            .map(|row| row[1 + col])
            .unwrap_or(key)
    }
}

#[cfg(windows)]
fn windows_ui_is_french() -> bool {
    // `GetUserDefaultUILanguage` without pulling a Windows API crate: the primary language id
    // of French is 0x0C.
    unsafe extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    (unsafe { GetUserDefaultUILanguage() } & 0x3ff) == 0x0c
}

#[cfg(not(windows))]
fn windows_ui_is_french() -> bool {
    false
}

/// `[key, English, French]`
const TABLE: &[[&str; 3]] = &[
    ["tab_library", "Animations", "Animations"],
    ["tab_behaviors", "Behaviors", "Comportements"],
    ["tab_settings", "Settings", "Paramètres"],
    ["add", "➕ Add…", "➕ Ajouter…"],
    ["remove", "🗑 Remove", "🗑 Retirer"],
    ["duplicate", "Duplicate", "Dupliquer"],
    ["drop_hint", "Drop images, GIFs, videos or packages here", "Déposez ici des images, GIF, vidéos ou paquets"],
    ["empty_library", "Your library is empty. Add an animation to put it on your desktop.", "Votre bibliothèque est vide. Ajoutez une animation pour la placer sur votre bureau."],
    ["select_item", "Select an animation on the left.", "Sélectionnez une animation à gauche."],
    ["name", "Name", "Nom"],
    ["source", "File", "Fichier"],
    ["change", "Change…", "Changer…"],
    ["show_on_desktop", "Show on desktop", "Afficher sur le bureau"],
    ["loading", "Loading…", "Chargement…"],
    ["sec_position", "Position", "Position"],
    ["center", "Center on screen", "Centrer à l'écran"],
    ["sec_look", "Appearance", "Apparence"],
    ["scale", "Size", "Taille"],
    ["opacity", "Opacity", "Opacité"],
    ["flip_h", "Mirror horizontally", "Miroir horizontal"],
    ["flip_v", "Mirror vertically", "Miroir vertical"],
    ["sec_playback", "Playback", "Lecture"],
    ["speed", "Speed", "Vitesse"],
    ["fps_override", "Force frame rate", "Forcer la fréquence d'images"],
    ["paused", "Paused", "En pause"],
    ["sec_behavior", "Behavior", "Comportement"],
    ["draggable", "Can be dragged with the mouse", "Déplaçable à la souris"],
    ["click_through", "Click-through (ignore the mouse)", "Traversable (ignore la souris)"],
    ["always_on_top", "Always on top", "Toujours au premier plan"],
    ["sec_audio", "Sound", "Son"],
    ["audio_file", "Sound file", "Fichier audio"],
    ["none", "None", "Aucun"],
    ["clear", "Clear", "Effacer"],
    ["video_sound", "Play the video's sound", "Lire le son de la vidéo"],
    ["volume", "Volume", "Volume"],
    ["sec_sprite", "Sprite sheet", "Planche de sprites"],
    ["is_sprite", "This image is a sprite sheet", "Cette image est une planche de sprites"],
    ["columns", "Columns", "Colonnes"],
    ["rows", "Rows", "Lignes"],
    ["frames", "Frames (0 = all)", "Images (0 = toutes)"],
    ["fps", "Frames per second", "Images par seconde"],
    ["sec_edit", "Edit / remove background", "Éditer / retirer le fond"],
    ["color_key", "Remove a color", "Retirer une couleur"],
    ["pick_corner", "Use corner color", "Couleur du coin"],
    ["tolerance", "Tolerance", "Tolérance"],
    ["softness", "Softness", "Adoucissement"],
    ["ai_remove", "✨ Remove background with AI", "✨ Retirer le fond avec l'IA"],
    ["ai_need_model", "Choose an ONNX model in Settings first (e.g. u2netp.onnx).", "Choisissez d'abord un modèle ONNX dans les paramètres (ex. u2netp.onnx)."],
    ["ai_unavailable", "This build was compiled without AI support.", "Cette version a été compilée sans l'IA."],
    ["ai_running", "Removing background…", "Retrait du fond…"],
    ["export_apng", "Export as APNG…", "Exporter en APNG…"],
    ["working", "Working…", "Traitement…"],
    ["done", "Done", "Terminé"],
    ["hide_all", "Hide all animations", "Masquer toutes les animations"],
    ["show_all", "Show all animations", "Afficher toutes les animations"],
    ["open_panel", "Open Open Anima", "Ouvrir Open Anima"],
    ["quit", "Quit", "Quitter"],
    ["language", "Language", "Langue"],
    ["autostart", "Start with Windows", "Lancer au démarrage de Windows"],
    ["close_to_tray", "Closing the window keeps running in the tray", "Fermer la fenêtre laisse l'appli dans la zone de notification"],
    ["master_volume", "Master volume", "Volume général"],
    ["ffmpeg", "ffmpeg (videos)", "ffmpeg (vidéos)"],
    ["ffmpeg_auto", "Found in PATH", "Trouvé dans le PATH"],
    ["ffmpeg_missing", "Not found: install ffmpeg to play MP4/WebM/MOV files.", "Introuvable : installez ffmpeg pour lire les fichiers MP4/WebM/MOV."],
    ["ai_model", "AI model (ONNX)", "Modèle IA (ONNX)"],
    ["ai_model_hint", "U²-Net family models work: u2netp.onnx (small, fast), u2net.onnx, silueta.onnx…", "Les modèles de la famille U²-Net fonctionnent : u2netp.onnx (petit, rapide), u2net.onnx, silueta.onnx…"],
    ["browse", "Browse…", "Parcourir…"],
    ["open_data_dir", "Open data folder", "Ouvrir le dossier de données"],
    ["about", "About", "À propos"],
    ["license_text", "Free software under the GNU Affero General Public License v3 or later.", "Logiciel libre sous licence GNU Affero General Public License v3 ou ultérieure."],
    ["behaviors_intro", "Behaviors are small programs that drive animations on your desktop (make a character walk along the screen edges, react to the time, …). They are distributed as .oapack packages.", "Les comportements sont de petits programmes qui pilotent des animations sur votre bureau (faire marcher un personnage le long des bords de l'écran, réagir à l'heure, …). Ils sont distribués sous forme de paquets .oapack."],
    ["load_package", "📦 Load package…", "📦 Charger un paquet…"],
    ["load_folder", "📁 Load folder…", "📁 Charger un dossier…"],
    ["create_package", "🛠 Create package…", "🛠 Créer un paquet…"],
    ["run", "▶ Run", "▶ Lancer"],
    ["stop", "■ Stop", "■ Arrêter"],
    ["running", "Running", "En cours"],
    ["stopped", "Stopped", "Arrêté"],
    ["confirm_run", "This package contains a program that will run on your computer with your permissions. Only run packages from authors you trust.", "Ce paquet contient un programme qui s'exécutera sur votre ordinateur avec vos droits. N'exécutez que des paquets d'auteurs de confiance."],
    ["command", "Command", "Commande"],
    ["author", "Author", "Auteur"],
    ["description", "Description", "Description"],
    ["log", "Log", "Journal"],
    ["remove_package", "Remove package", "Supprimer le paquet"],
    ["pick_folder_for_package", "Choose the behavior folder (it must contain a manifest.toml or an executable and a temp folder)", "Choisissez le dossier du comportement (il doit contenir un manifest.toml ou un exécutable et un dossier temp)"],
    ["package_created", "Package created", "Paquet créé"],
    ["error", "Error", "Erreur"],
    ["cancel", "Cancel", "Annuler"],
    ["edit", "Edit", "Éditer"],
];
