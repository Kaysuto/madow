# Madow

> Le poste de travail tout-en-un des développeurs, avec le modèle d'IA de leur choix.

**Madow** (de 窓, *mado*, « fenêtre » en japonais) réunit mails, agenda, tâches, demandes de fusion, tickets, agents de code et actus dans une seule application de bureau, qui garde les données sur votre machine. Le modèle est au choix : un abonnement déjà payé, une clé API, une offre gratuite ou un modèle qui tourne sur votre ordinateur.

Aperçu : Madow s'utilise au quotidien mais reste jeune, et des choses bougeront d'une version à l'autre.

## Ce qui marche aujourd'hui

- **Aujourd'hui** : la journée en un coup d'œil, avec un briefing écrit par votre modèle.
- **Boîte** : mails, demandes de relecture, notifications et alertes dans une seule liste, triée par votre modèle rapide. Répondez à un mail à partir d'un brouillon que vous corrigez avant l'envoi.
- **Tâches** : vos tâches et les tickets de vos outils de suivi, ensemble.
- **Agenda** : vos agendas par mois, et de nouveaux événements écrits en langage naturel.
- **Code** : vos dépôts git avec leur état, leurs demandes de fusion, tickets et alertes de sécurité, et les sessions d'agents de code qui tournent sur votre machine. Lancez un agent sur un ticket, dans un terminal où vous le voyez travailler.
- **Veille** : les articles des flux que vous suivez, à lire maintenant, plus tard ou jamais.
- **Notes** : des notes en Markdown, libres ou rattachées à une tâche, un ticket, un mail ou un événement, retrouvées par la recherche.
- **Bilan** : le temps de focus par projet, vos commits, et un bilan écrit du jour ou de la semaine.
- **Assistant** : un chat qui connaît votre journée, avec son historique.
- **Focus** : un chrono lancé depuis n'importe quel élément.
- **Automatisations** : vos règles sur ce qui arrive (« un mail contenant facture », « une alerte ») : en faire une tâche, recevoir une notification, ou le sortir de la Boîte. Madow suggère une règle quand vous refaites le même geste sur ce qu'un expéditeur envoie. Les règles n'agissent que dans Madow.
- **Lanceur** : Ctrl+K, ou Alt+Espace depuis n'importe où sur l'ordinateur, retrouve un élément, change d'espace, ouvre un projet dans votre éditeur, note une tâche ou une idée, ou interroge l'assistant. Le raccourci global se change dans les réglages.
- **Barre d'état** : processeur, mémoire et disque, la musique en cours avec ses boutons (Windows et Linux), les agents au travail, et les serveurs qui ne répondent plus.

Sources pour l'instant : toute boîte mail (IMAP/SMTP), les agendas par lien iCal, GitHub et GitLab, Linear et Jira, les flux RSS et Atom, et des serveurs à surveiller. Modèles : Claude Code ou Codex avec votre abonnement, une clé API (Anthropic ou tout service compatible OpenAI), ou un modèle local. Des données d'exemple sont intégrées, pour faire le tour avant de connecter quoi que ce soit.

Vos données restent sur votre machine. Le modèle n'a jamais d'outil dessus : Madow lit vos sources elle-même et lui donne du texte, et rien n'est envoyé ni créé sans votre clic.

## Installation

Téléchargez l'installeur de votre système depuis la [dernière version](https://github.com/Kaysuto/madow/releases).

Les installeurs ne sont pas encore signés, votre système affichera donc un avertissement :

- **Windows** : sur l'avertissement SmartScreen, choisissez « Informations complémentaires », puis « Exécuter quand même ».
- **macOS** : ouvrez l'application une fois, puis autorisez-la dans Réglages Système → Confidentialité et sécurité → « Ouvrir quand même ». Prenez `aarch64` pour Apple Silicon, `x64` pour Intel.
- **Linux** : installez le `.deb` ou le `.rpm`, ou rendez le `.AppImage` exécutable et lancez-le.

[English version](README.md)

## Développement

Prérequis : [Rust](https://rustup.rs), Node.js 24, [pnpm](https://pnpm.io) et les [dépendances système de Tauri](https://tauri.app/start/prerequisites/) pour votre plateforme.

```sh
pnpm install
pnpm tauri dev    # lance l'application avec rechargement à chaud
pnpm check        # vérifie les types de l'interface
pnpm test         # tests unitaires
```

L'interface est dans `src/` (Svelte 5 + TypeScript) et le cœur dans `src-tauri/` (Rust).

## Licence

[MIT](LICENSE)
