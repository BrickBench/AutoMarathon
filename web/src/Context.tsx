import {createContext} from 'react'
import { AuthState, StreamEntry, StreamRunnersEntry, WebUIState } from './websocket';

export type WebUIStateContext = {
    webuistate: WebUIState
    setWebUIState:(c: WebUIState) => void
}

export const WebUIStateContext = createContext<WebUIStateContext>({webuistate:new WebUIState(),setWebUIState:(c: WebUIState) => {}});

export type AuthStateContext = {
    authState: AuthState
    setAuthState:(c: AuthState) => void
}

export const AuthStateContext = createContext<AuthStateContext>({authState: new AuthState(),setAuthState:(c: AuthState) => {}});


export type StreamStateContext = {
    streamContext: [StreamEntry | undefined, (stream: StreamEntry | undefined) => void]
    selectedLayoutContext: [string, (layout: string) => void]
}

export const StreamStateContext = createContext<StreamStateContext>({streamContext: [new StreamEntry(), (_:StreamEntry)=>{}], selectedLayoutContext: ["", (_:string)=>{}]});
