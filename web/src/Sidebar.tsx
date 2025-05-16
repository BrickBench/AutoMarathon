import { useState, useContext } from 'react'
import { Row, Col, Button, Accordion, ListGroup, Badge, InputGroup, FormControl, DropdownButton, DropdownItem } from 'react-bootstrap';
import { AMState, Person, Runner, Event, StreamHost, WebUIState, EventTab, StreamEntry } from './websocket';
import { WebUIStateContext } from './Context';
import { Search } from 'react-bootstrap-icons';
import { doPost } from './Api';

function EventList({ searchField, events, people, streams, hosts }: {
  searchField: string, events: Event[], people: Map<number, Person>, streams: StreamEntry[],
  hosts: Map<string, StreamHost>
}) {
  const filteredEvents = events.filter((event) => {
    return !searchField || searchField.length == 0 || event.name.toLowerCase().includes(searchField.toLowerCase());
  });

  return <ListGroup className='overflow-auto'>
    {filteredEvents.map(event => <EventItem key={event.id} event={event} people={people} streams={streams} hosts={hosts}></EventItem>)}
  </ListGroup>
}

function PeopleList({ searchField, people }: { searchField: string, people: Map<number, Person> }) {
  const filteredPeople = [...people.values()].filter((person) => {
    return !searchField || searchField.length == 0 || person.name.toLowerCase().includes(searchField.toLowerCase());
  }).sort((a, b) => a.name.localeCompare(b.name));

  return <ListGroup>
    {filteredPeople.map((person) => <PersonItem key={person.id} person={person}></PersonItem>)}
  </ListGroup>
}

function EventItem({ event, people, streams, hosts }: { event: Event, people: Map<number, Person>, streams: StreamEntry[], hosts: Map<string, StreamHost> }) {
  const { webuistate, setWebUIState } = useContext(WebUIStateContext);

  let live = streams.some(stream => { return stream.event == event.id });
  let players = Object.entries(event.runner_state).map(([_, value]) => people.get(value.runner)?.name ?? "Unknown");
  let truncatedPlayers = (players.length > 4 ? [...players.slice(0, 4), "and " + (players.length - 4).toString() + " more"] : players);

  return (
    <ListGroup.Item active={webuistate.selectedEvent == event.id} onClick={() => {
      let hostNames = [...hosts.entries()];
      let bestHostName = webuistate.selectedHost;
      if (hostNames.length > 0 && webuistate.selectedHost.length == 0) {
        for (let [key, host] of hostNames) {
          if (host.connected) {
            bestHostName = key;
            break;
          }
        }
      }

      setWebUIState({
        ...webuistate,
        selectedEvent: event.id,
        selectedPerson: -1,
        selectedHost: bestHostName
      });
    }}>
      <div className="d-flex w-100 justify-content-between">
        <b className="mb-1 fs-8">{event.name}</b>
        <small>{event.event_start_time && timeAgo(new Date(event.event_start_time))}</small>
      </div>
      {truncatedPlayers.map((player, index) => <Badge key={index} pill bg="light-subtle">{player}</Badge>)}
      {live && <Badge pill bg="success">Live</Badge>}
      {event.complete && <Badge pill bg="primary">Complete</Badge>}
    </ListGroup.Item>
  );
}

function PersonItem({ person }: { person: Person }) {
  const { webuistate, setWebUIState } = useContext(WebUIStateContext);
  return (
    <ListGroup.Item active={webuistate.selectedPerson == person.id} onClick={() => {
      setWebUIState({
        ...webuistate,
        selectedPerson: person.id,
        selectedEvent: -1,
        eventSubmenu: EventTab.Setup
      });
    }}>
      <b>{person.name}</b>
    </ListGroup.Item>
  );
}

export function Sidebar({ people, events, streams, hosts }: { people: Map<number, Person>, events: Event[], streams: StreamEntry[], hosts: Map<string, StreamHost> }) {
  const [searchBarText, setSearchBarText] = useState<string>("");
  const [createField, setCreateField] = useState<string>("");

  return <Col lg={2}>
    <Row>
      <InputGroup>
        <InputGroup.Text id="basic-addon1"><Search /></InputGroup.Text>
        <FormControl type="text" onChange={(e: React.ChangeEvent<HTMLInputElement>) => setSearchBarText(e.target.value)}
          value={searchBarText} />
      </InputGroup>
    </Row>
    <Row className='pt-2'>
      <h4>Add new</h4>
    </Row>
    <Row>
      <InputGroup className="mb-3">
        <FormControl aria-label="Name" type='text' value={createField} onChange={(e) => {
          setCreateField(e.currentTarget.value);
        }} />
        <DropdownButton variant="success" title="Add" id="input-group-dropdown-2" align="end">
          <DropdownItem onClick={() => {
            if (createField.trim() != "") {
              doPost('participant', 'POST', {
                id: -1,
                name: createField,
                volume_percent: 50,
                host: false
              });
              setCreateField("");
            }else{
              alert("Please enter a name for the person that will be created.");
            }
          }}>Person</DropdownItem>
          <DropdownItem onClick={() => {
            if (createField.trim() != "") {
              doPost('event', 'POST', {
                id: -1,
                name: createField,
                game: "",
                category: "",
                console: "",
                runner_state: {},
                is_relay: false,
                is_marathon: false,
                preferred_layouts: [],
                complete: false,
                estimate: null,
                tournament: null,
                therun_race_id: null,
                event_start_time: null,
                timer_start_time: null,
                timer_end_time: null,
                commentators: []
              });
              setCreateField("");
            }else{
              alert("Please enter a name for the event that will be created.");
            }
          }}>Event</DropdownItem>
        </DropdownButton>
      </InputGroup>
    </Row>
    <Row className='pt-1'>
      <Accordion defaultActiveKey="0">
        <Accordion.Item eventKey="0">
          <Accordion.Header>Events</Accordion.Header>
          <Accordion.Body className='p-0'>
            <EventList searchField={searchBarText} events={events} people={people} streams={streams} hosts={hosts}></EventList>
          </Accordion.Body>
        </Accordion.Item>
      </Accordion>
    </Row>
    <Row className='pt-3'>
      <Accordion defaultActiveKey="0">
        <Accordion.Item eventKey="0">
          <Accordion.Header>People</Accordion.Header>
          <Accordion.Body className='p-0'>
            <PeopleList searchField={searchBarText} people={people}></PeopleList>
          </Accordion.Body>
        </Accordion.Item>
      </Accordion>
    </Row>
  </Col>
}

function timeAgo(input: Date) {
  const date = (input instanceof Date) ? input : new Date(input);
  const formatter = new Intl.RelativeTimeFormat('en');
  const ranges = {
    years: 3600 * 24 * 365,
    months: 3600 * 24 * 30,
    weeks: 3600 * 24 * 7,
    days: 3600 * 24,
    hours: 3600,
    minutes: 60,
    seconds: 1
  };
  const secondsElapsed = (date.getTime() - Date.now()) / 1000;
  for (let key in ranges) {
    if (ranges[key] < Math.abs(secondsElapsed)) {
      const delta = secondsElapsed / ranges[key];
      return formatter.format(Math.round(delta), key);
    }
  }
}
