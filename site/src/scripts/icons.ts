// Every icon the site draws, as SVG markup, from three sources:
//  - Fluent UI System Icons (MIT): Microsoft's open-source set, the same family as the
//    Segoe Fluent Icons font the app uses (which can't be shipped on a website).
//  - The app's own marks in design/assets: the agents' logos and ReUI's device drawings.
//  - Simple Icons (CC0) for third-party brands (apps in a call, players, browsers); brand
//    names and logos belong to their owners and only say what ReUI works with.
import paste from '@fluentui/svg-icons/icons/clipboard_paste_20_regular.svg?raw';
import box from '@fluentui/svg-icons/icons/box_20_regular.svg?raw';
import timer from '@fluentui/svg-icons/icons/timer_20_regular.svg?raw';
import calendar from '@fluentui/svg-icons/icons/calendar_ltr_20_regular.svg?raw';
import devices from '@fluentui/svg-icons/icons/phone_laptop_20_regular.svg?raw';
import pulse from '@fluentui/svg-icons/icons/pulse_20_regular.svg?raw';
import music from '@fluentui/svg-icons/icons/music_note_2_20_regular.svg?raw';
import mic from '@fluentui/svg-icons/icons/mic_20_regular.svg?raw';
import alert from '@fluentui/svg-icons/icons/alert_20_regular.svg?raw';
import check from '@fluentui/svg-icons/icons/checkmark_20_regular.svg?raw';
import dismiss from '@fluentui/svg-icons/icons/dismiss_20_regular.svg?raw';
import download from '@fluentui/svg-icons/icons/arrow_download_20_regular.svg?raw';
import play from '@fluentui/svg-icons/icons/play_20_filled.svg?raw';
import pause from '@fluentui/svg-icons/icons/pause_20_filled.svg?raw';
import previous from '@fluentui/svg-icons/icons/previous_20_filled.svg?raw';
import next from '@fluentui/svg-icons/icons/next_20_filled.svg?raw';
import speaker from '@fluentui/svg-icons/icons/speaker_2_20_regular.svg?raw';
import search from '@fluentui/svg-icons/icons/search_20_regular.svg?raw';
import document from '@fluentui/svg-icons/icons/document_20_regular.svg?raw';
import question from '@fluentui/svg-icons/icons/question_circle_20_regular.svg?raw';
import terminal from '@fluentui/svg-icons/icons/window_console_20_regular.svg?raw';
import plan from '@fluentui/svg-icons/icons/task_list_ltr_20_regular.svg?raw';
import globe from '@fluentui/svg-icons/icons/globe_20_regular.svg?raw';
import cast from '@fluentui/svg-icons/icons/speaker_bluetooth_20_regular.svg?raw';
import bin from '@fluentui/svg-icons/icons/delete_20_regular.svg?raw';

import claude from '../../../design/assets/agents/claude.svg?raw';
import openai from '../../../design/assets/agents/openai.svg?raw';
import opencode from '../../../design/assets/agents/opencode.svg?raw';
import earbuds from '../../../design/assets/devices/earbuds-small.svg?raw';
import headphones from '../../../design/assets/devices/headphones-small.svg?raw';

import zoom from 'simple-icons/icons/zoom.svg?raw';
import discord from 'simple-icons/icons/discord.svg?raw';
import webex from 'simple-icons/icons/webex.svg?raw';
import whatsapp from 'simple-icons/icons/whatsapp.svg?raw';
import telegram from 'simple-icons/icons/telegram.svg?raw';
import signal from 'simple-icons/icons/signal.svg?raw';
import spotify from 'simple-icons/icons/spotify.svg?raw';
import vlc from 'simple-icons/icons/vlcmediaplayer.svg?raw';
import chrome from 'simple-icons/icons/googlechrome.svg?raw';
import firefox from 'simple-icons/icons/firefoxbrowser.svg?raw';
import brave from 'simple-icons/icons/brave.svg?raw';
import opera from 'simple-icons/icons/opera.svg?raw';
import github from 'simple-icons/icons/github.svg?raw';

export const icons = {
  paste, box, timer, calendar, devices, pulse, music, mic, alert, check, dismiss, download, play, pause, previous, next,
  speaker, search, document, question, terminal, plan, globe, cast, bin,
  claude, openai, opencode, earbuds, headphones,
  zoom, discord, webex, whatsapp, telegram, signal, spotify, vlc, chrome, firefox, brave, opera, github,
};
export type IconName = keyof typeof icons;

/** Each brand's own colour, for logos shown in colour. */
export const brand: Partial<Record<IconName, string>> = {
  zoom: '#0B5CFF', discord: '#5865F2', webex: '#000000', whatsapp: '#25D366', telegram: '#26A5E4', signal: '#3A76F0',
  spotify: '#1ED760', vlc: '#FF8800', chrome: '#4285F4', firefox: '#FF7139', brave: '#FB542B', opera: '#FF1B2D',
};
