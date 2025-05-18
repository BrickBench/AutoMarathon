import { useState, useEffect, createContext, useContext, useRef } from 'react'
import { Container, Row, Col, Button, Accordion, ListGroup } from 'react-bootstrap';
import { AMState, Person, Runner, Event, StreamHost, WebUIState, diffAMState, LockState, AuthState, StreamEntry, EventTab, CustomFields } from './websocket';
import { Sidebar } from './Sidebar'
import { AuthStateContext, WebUIStateContext } from './Context';
import { EditPerson } from './EditPerson';
import { EditEvent } from './EditEvent';
import { LoginPage } from './Login';
import { HostSelector, PanelSelector } from './TopBar';
import { HostPanelLock } from './HostPanelLock';
import { HostPanel } from './hostpanel/HostPanel';
import { Dashboard } from './dashboard/Dashboard';
import { NotificationsToast, ToastNotificationsState, ToastNotifStateContext } from './AMNotification';
import { EditEventData } from './dataeditor/EventData';

function createWebSocket(path: string, isCreated: boolean): WebSocket | undefined {
  if (isCreated) {
    return undefined;
  }
  var protocolPrefix = (window.location.protocol === 'https:') ? 'wss:' : 'ws:';
  return new WebSocket(new URL(path, protocolPrefix + '//' + location.hostname + ":28010"));
}

function AMApp() {
  const [webSocketCreated, setWebSocketCreated] = useState<boolean>(false);
  const [webSocket, setWebSocket] = useState<WebSocket | undefined>(createWebSocket("/ws", webSocketCreated));
  const [editorWebSocketCreated, setEditorWebSocketCreated] = useState<boolean>(false);
  const [editorWebSocket, setEditorWebSocket] = useState<WebSocket | undefined>(createWebSocket("/ws/dashboard-editor", editorWebSocketCreated));

  const [lockOwner, setLockOwner] = useState<LockState>();

  const [people, setPeople] = useState<Map<number, Person>>(new Map());
  const [runners, setRunners] = useState<Map<number, Runner>>(new Map());
  const [events, setEvents] = useState<Event[]>([]);
  const [streams, setStreams] = useState<StreamEntry[]>([]);
  const [hosts, setHosts] = useState<Map<string, StreamHost>>(new Map());
  const [customFields, setCustomFields] = useState<CustomFields>({});

  const [webuistate, setWebUIState] = useState<WebUIState>(new WebUIState());

  const { authState, setAuthState } = useContext(AuthStateContext);

  const [notification, setNotification] = useState(new ToastNotificationsState());

  function releaseLock() {
    if (!lockOwner) {
      return;
    }
    if (authState.username == lockOwner.editor) {
      let lock: LockState = { editor: null, unix_time: 0 };
      editorWebSocket?.send(JSON.stringify(lock));
    }
  }

  useEffect(() => {
    if(webSocket){
      webSocket.onopen = (event) => {
        setWebSocketCreated(true);
      };

      webSocket.onmessage = (event) => {
        var state: AMState = JSON.parse(event.data);
        console.log(state);
        diffAMState(state, people, setPeople, runners, setRunners, events, setEvents, hosts, setHosts, streams, setStreams,
          customFields, setCustomFields
        );
      };

      webSocket.onclose = function(event) {
        setWebSocketCreated(false);
      };

      webSocket.onerror = function(err) {
        setWebSocketCreated(false);
      };
    }

    return () => {
    };
  }, [webSocket]);

  useEffect(() => {
    if(editorWebSocket){
      editorWebSocket.onopen = (event) => {
        setEditorWebSocketCreated(true);
      };

      editorWebSocket.onmessage = function(event) {
        var state: LockState = JSON.parse(event.data);
        setLockOwner(state);
      };

      editorWebSocket.onclose = function(event) {
        setEditorWebSocketCreated(false);
      };

      editorWebSocket.onerror = function(err) {
        setEditorWebSocketCreated(false);
      };
    }

    return () => {
    };
  }, [editorWebSocket]);

  let personSelected = people.get(webuistate.selectedPerson);
  let selectedEvent = events.find((e) => e.id == webuistate.selectedEvent)!;

  return webSocket && editorWebSocket && webSocket.readyState == WebSocket.OPEN ? <WebUIStateContext.Provider value={{ webuistate: webuistate, setWebUIState: setWebUIState }}>
    <ToastNotifStateContext.Provider value={{ toastNotifState: notification, setToastNotifState: setNotification }}>
      <Container fluid={true}>
        <Row>
          <Sidebar people={people} events={events} streams={streams} hosts={hosts}></Sidebar>

          <Col lg={10}>
            {webuistate.selectedPerson != -1 && <EditPerson person={personSelected} runner={runners.get(webuistate.selectedPerson)} events={events}>
            </EditPerson>}
            {webuistate.selectedEvent != -1 &&
              <>
                <Row className="border-bottom">
                  <Col lg={9}>
                    <PanelSelector selectedEvent={selectedEvent} releaseLock={releaseLock}></PanelSelector>
                  </Col>
                  <Col lg={3}>
                    <HostSelector hosts={hosts} releaseLock={releaseLock}></HostSelector>
                  </Col>
                </Row>
                {webuistate.eventSubmenu == EventTab.Setup &&
                  <EditEvent event={selectedEvent} people={people} runners={runners} />
                }
                {webuistate.eventSubmenu == EventTab.Data &&
                  <EditEventData event={selectedEvent} people={people}/>
                }
                {webuistate.eventSubmenu == EventTab.Stream ?
                  (lockOwner && lockOwner.editor == authState.username ?
                    <HostPanel host={hosts.get(webuistate.selectedHost)!} events={events} people={people} streams={streams} runners={runners} customFields={customFields}></HostPanel> :
                    <HostPanelLock authState={authState} lockState={lockOwner} lockSocket={editorWebSocket}></HostPanelLock>) :
                  <></>
                }
                {webuistate.eventSubmenu == EventTab.Dashboard && selectedEvent ?
                  <Dashboard event={selectedEvent}></Dashboard>
                  :
                  <></>}
              </>
            }
          </Col>

        </Row>
        <NotificationsToast />
      </Container>
    </ToastNotifStateContext.Provider>
  </WebUIStateContext.Provider> : <Row className="justify-content-center align-items-center" style={{ height: 600 }}>
    <Col md={12} className="d-flex justify-content-center"><h1>No Connection</h1></Col></Row>
}

function App() {
  const [authState, setAuthState] = useState<AuthState>();//{"token":"token","username":"testuser"});

  return (
    authState?.token ? <AuthStateContext.Provider value={{ authState: authState, setAuthState: setAuthState }}>
      <AMApp></AMApp>
    </AuthStateContext.Provider> : <LoginPage beginSession={setAuthState}></LoginPage>
  )
}

export default App
