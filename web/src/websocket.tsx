export interface AMState {
    streams: StreamEntry[];
    events: Event[];
    people: {
        [key: string]: Person;
    }
    runners: {
        [key: string]: Runner;
    }
    hosts: {
        [key: string]: StreamHost;
    }
    custom_fields: CustomFields;
}

export type CustomFields = {
    [key: string]: string;
}

export type RunnerStateEntry = {
    runner: number;
    result: string;
};

export type RunnerState = {
    [key: string]: RunnerStateEntry;
}

export interface Event {
    id: number;
    name: string;
    game?: string;
    category?: string;
    console?: string;
    complete?: boolean;
    estimate?: number;
    tournament?: number;
    therun_race_id?: string;
    event_start_time?: number;
    timer_start_time?: number;
    timer_end_time?: number;
    preferred_layouts: string[];
    is_relay: boolean;
    is_marathon: boolean;
    commentators: number[];
    runner_state: RunnerState;
}

export type StreamRunnersEntry = {
    [key: string]: number
};

export class StreamEntry {
    event: number = -1;
    obs_host: string = "";
    audible_runner?: number = -1;
    requested_layout?: string = "";
    stream_runners: StreamRunnersEntry = {};
}

export interface Person {
    id: number;
    name: string;
    pronouns?: string;
    location?: string;
    discord_id?: number;
    host: boolean;
}

export interface Runner {
    participant: number;
    stream?: string;
    therun?: string;
    override_stream_url?: string;
    stream_urls?: {
        [key: string]: string;
    };
    stream_volume_percent: number;
}

export interface StreamHost {
    connected: boolean;
    streaming: boolean;
    stream_frame_rate: number;
    program_scene: string;
    preview_scene?: string;
    scenes: {
        [key: string]: StreamScene;
    };
    discord_users: {
        [key: string]: DiscordUser;
    };
}

export interface VoiceCallStatus {
    username: string;
    speaking: boolean;
    peak_volume: number;
    desired_volume: number;
}

export interface DiscordUser {
    username: string;
    id: number;
    volume_percent: number;
    participant?: number;
}

export interface StreamScene {
    name: string;
    active: boolean;
    sources: {
        [key: string]: StreamSource[];
    }
}

export interface StreamSource {
    name: string;
    x: number;
    y: number;
    width: number;
    height: number;
    crop_left: number;
    crop_right: number;
    crop_top: number;
    crop_bottom: number;
}

export enum EventTab {
    Setup,
    Stream,
    Data,
    Dashboard
}

export class WebUIState {
    selectedEvent: number = -1;
    selectedPerson: number = -1;
    selectedHost: string = "";
    eventSubmenu: EventTab = EventTab.Setup;
}

export class AuthState {
    username: string | undefined;
    token: string | undefined;
}

export class LockState {
    editor: string | null = null;
    unix_time: number = 0;
}

export function diffAMState(newData: AMState, people: Map<number, Person>, setPeople: (state: Map<number, Person>) => void,
    runners: Map<number, Runner>, setRunners: (state: Map<number, Runner>) => void,
    events, setEvents,
    hosts, setHosts: (state: Map<string, StreamHost>) => void,
    streams: StreamEntry[], setStreams: (stream: StreamEntry[]) => void,
    customFields: CustomFields, setCustomFields: (customFields : CustomFields) => void) {

    var newPeopleMap = new Map<number, Person>(Object.entries(newData.people).map(([key, value]) => [parseInt(key), value]));
    setPeople(newPeopleMap);

    var newRunnersMap = new Map<number, Runner>(Object.entries(newData.runners).map(([key, value]) => [parseInt(key), value]));
    setRunners(newRunnersMap);

    var newEvents = newData.events;
    setEvents(newEvents);

    var newHosts = new Map<string, StreamHost>(Object.entries(newData.hosts));
    setHosts(newHosts);

    var newstream = newData.streams;
    setStreams(newstream);

    var newCustomField = newData.custom_fields;
    setCustomFields(newCustomField);

    //var peopleChanges = mapsAreEqual(newPeopleMap, people);
}

