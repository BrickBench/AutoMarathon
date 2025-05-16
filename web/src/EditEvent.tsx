import { useState, useContext, useEffect} from 'react'
import {Row, Col, Button, Accordion, ListGroup, Form, FormLabel, FormControl, InputGroup} from 'react-bootstrap';
import {AMState, Person, Runner, Event, StreamHost, WebUIState, EventTab} from './websocket';
import Creatable from 'react-select/creatable';
import Select from 'react-select';
import { customStyles } from './Globals';
import { WebUIStateContext } from './Context';
import { doPost } from './Api'

function EventDataFieldInput({typename,customTemp,key2,setCustomTemp,horizontal,runner,event,setEventTemp}: {typename:any,customTemp: any,key2:any,setCustomTemp:any,horizontal:boolean,event:any,runner:any,setEventTemp:any}){
    const [inputState,setInputState]  = useState(customTemp[key2]);
    useEffect(() => {
        setInputState(customTemp[key2]);
    }, [customTemp]);
    return horizontal ? <div className="input-group">
      <label className="input-group-text">{key2}</label>
      <input type="text" name={key2} className="form-control" onChange={({ target }) => {
          setInputState(target.value);
          let temp = customTemp;
          temp[key2] = target.value;
          let out = {};
          out[typename] = temp;
          setCustomTemp(out);
          let tempevent = {...event};
          tempevent.runner_state[runner].result = out;
          setEventTemp(tempevent);
        }} value={inputState || ''}/>
    </div> : 
    <div className="col-2">
    <label>{key2}</label>
    <input type="text" name={key2} className="form-control" onChange={({ target }) => {
        setInputState(target.value);
        let temp = customTemp;
        temp[key2] = target.value;
        let out = {};
        out[typename] = temp;
        setCustomTemp(out);
        let tempevent = {...event};
          tempevent.runner_state[runner].result = out;
          setEventTemp(tempevent);
      }} value={inputState || ''}/>
  </div>;
  }
  
  function EventRunnerData({runner,event,setEventState,data}:{runner:any,event:any,setEventState:any,data:any}){
  const {webuistate, setWebUIState } = useContext(WebUIStateContext);
  const { amstate, setAMState } = useContext(AMStateContext);
  const [eventRunnerState,setEventRunnerStateTemp] = useState(data ? data : 
    {"SingleScore":{score: ""}});
  useEffect(() => {
    setEventRunnerStateTemp(data ? data : 
      {"SingleScore":{score: ""}});
  }, [webuistate,data])
  
  return (
    <div className="row pt-2">
        <div className="col-2">
            {amstate.runners[runner].name}
        </div>
        <div className="col-10">
          {eventRunnerState["SingleScore"] &&
            <div className="row">
            {Object.entries(eventRunnerState['SingleScore'])
            .map(([key,val])=>{return <EventDataFieldInput typename={"SingleScore"} runner={runner} horizontal={true} customTemp={eventRunnerState['SingleScore']} event={event} setEventTemp={setEventState} key2={key} setCustomTemp={setEventRunnerStateTemp}></EventDataFieldInput>})}
            </div>
          }
        </div>
    </div>
  );
  }
  
  function EditEventData({event}: {event: any}){
    const [eventState, setEventState] = useState({...event});
    const { amstate, setAMState } = useContext(AMStateContext);
    useEffect(() => {
      setEventState(event)
    }, [event]);
    return (<div className="container">
      <div className="row pt-2">
        <div className="col">
          <h2>Edit Event Data</h2>
        </div>
        <div className="col align-items-center me-auto">
          <button className="btn btn-primary" onClick={() => {
                doPost('http://'+apiurl+'/event','PUT', eventState);
            }}>Save Changes</button>
        </div>
      </div>
      {Object.entries(eventState.runner_state).map(([key,val])=><EventRunnerData runner={key} event={eventState} setEventState={setEventState} data={val.result}></EventRunnerData>)}
      </div>);
  }

  function HMSInput({time, onChangeCallback} : {time: number | undefined, onChangeCallback : (time:number) => void}){

    function getHMS(totalSeconds:number){
        let hours = Math.floor(totalSeconds / 3600);
        let minutes = Math.floor(Math.floor(totalSeconds / 60) % 60);
        let seconds = Math.floor(totalSeconds % 60);
        return [hours,minutes,seconds];
    }

    function hmsToSeconds(hms:number[]){
        return hms[0] * 3600 + hms[1] * 60 + hms[2];
    }

    return <InputGroup>
        <InputGroup.Text>Hours</InputGroup.Text>
        <FormControl type="number" min="0" max="60" placeholder="0" aria-label="hoursest" 
            onChange={e => {
                if(parseInt(e.target.value) >= 0 && parseInt(e.target.value) <= 60){
                    let hms = time ? getHMS(time) : [0,0,0];
                    hms[0] = parseInt(e.target.value);
                    onChangeCallback(hmsToSeconds(hms));
                }
            }} value={time ? Math.floor(time / 3600) : 0}/>
        <InputGroup.Text>Minutes</InputGroup.Text>
        <FormControl type="number" min="0" max="60" placeholder="0" aria-label="minest"
            onChange={e => {
                if(parseInt(e.target.value) >= 0 && parseInt(e.target.value) <= 60){
                    let hms = time ? getHMS(time) : [0,0,0];
                    hms[1] = parseInt(e.target.value);
                    onChangeCallback(hmsToSeconds(hms));
                }
            }} value={time ? Math.floor(Math.floor(time / 60) % 60) : 0}/>
        <InputGroup.Text>Seconds</InputGroup.Text>
        <FormControl type="number" min="0" max="60" placeholder="0" aria-label="secest"
            onChange={e => {
                if(parseInt(e.target.value) >= 0 && parseInt(e.target.value) <= 60){
                    let hms = time ? getHMS(time) : [0,0,0];
                    hms[2] = parseInt(e.target.value);
                    onChangeCallback(hmsToSeconds(hms));
                }
            }} value={time ? Math.floor(time % 60) : 0}/>
    </InputGroup>;
  }
  
  export function EditEvent({event, people, runners}: {event: Event | undefined, people: Map<number,Person>, runners: Map<number,Runner>}){
    const [eventState, setEventState] = useState(event);
    const { webuistate, setWebUIState } = useContext(WebUIStateContext);

    useEffect(()=>{
        setEventState(event);
    }, [event]);
  
    let runner_options =  [...runners.entries()].map(([key, value]) => ({value: key.toString(), label: people.get(value.participant)!.name}));
    let commentatorOptions =  [...people.entries()].map(([key, value]) => ({value: key, label: value.name}));
  
    return (eventState && <>
    <Form onSubmit={(event) => {
        event.preventDefault();
        doPost('event','PUT', eventState);              
      }}>
      <div className="mb-3">
        <Form.Label htmlFor="eventeditname">Name</Form.Label>
        <Form.Control type="text" id="eventeditname" onChange={e => setEventState({
                ...eventState,
                name: e.target.value
              })} value={eventState.name || ''}/>
      </div>
      <Row className="mb-3">
        <Col>
            <FormLabel htmlFor="runnereditgame">Game</FormLabel>
            <FormControl type="text" id="runnereditgame" onChange={e => setEventState({
                ...eventState,
                game: e.target.value
                })} value={eventState.game || ''}/>
        </Col>
        <Col>
            <Form.Label htmlFor="runnereditcategory">Category</Form.Label>
            <Form.Control type="text" id="runnereditcategory" onChange={e => setEventState({
                ...eventState,
                category: e.target.value
            })} value={eventState.category || ''}/>
        </Col>
        <Col>
            <Form.Label htmlFor="runnereditplatform">Platform</Form.Label>
            <Form.Control type="text" id="runnereditplatform" onChange={e => setEventState({
                ...eventState,
                console: e.target.value
            })} value={eventState.console || ''}/>
        </Col>
      </Row>
      <div className="mb-3">
        <Form.Label htmlFor="eventeditrunners">Runners</Form.Label>
        <Select styles={customStyles}  value={Array.from(Object.entries(eventState.runner_state), ([key, value]) => ({value: key, label: people.get(value.runner)!.name}))} 
        onChange={selectedOptions => {
          let temp_runners = {}
  
          for(const entry of selectedOptions){
              if(entry.value in eventState.runner_state){
                  temp_runners[entry.value] = eventState.runner_state[entry.value];
              }else{
                  temp_runners[entry.value] = {"runner": parseInt(entry.value),"result":null};
              }
          }
  
          setEventState({
            ...eventState,
            runner_state: temp_runners
          });
        }}
        options={runner_options} id="eventeditrunners" isMulti={true}></Select>
      </div>
      <Row className="mb-3">
        <Col>
            <FormLabel>Event Estimate</FormLabel>
            <HMSInput time={eventState.estimate} onChangeCallback={(time)=>{setEventState({...eventState, estimate: time})}}></HMSInput>
        </Col>
        <Col>
            <Form.Label htmlFor="runnerediteventstart">Event Start Time</Form.Label>
            <FormControl type="datetime-local" id="runnerediteventstart" onChange={e => {
                if(e.target.value){
                    let date_entry = new Date(e.target.value);
                    let utc = date_entry.getTime();
                    setEventState({
                        ...eventState,
                        event_start_time: utc
                    });
                }
            }} value={eventState.event_start_time ? (new Date(new Date(eventState.event_start_time).getTime() - new Date(eventState.event_start_time).getTimezoneOffset() * 60000).toISOString()).slice(0, -1) : ''}/>
        </Col>
      </Row>
      <Row className="mb-3">
        <Form.Label>Preferred Layouts</Form.Label>
        <Creatable isMulti={true} value={eventState.preferred_layouts.map((value:string) => ({value: value, label: value}))} 
        onChange={selectedOptions => {
          let temp_layouts = []
          for(const entry of selectedOptions){
            temp_layouts.push(entry['value']);
          }
          setEventState({
            ...eventState,
            preferred_layouts: temp_layouts
          });
        }}></Creatable>
      </Row>
      <Row className="mb-3">
        <Form.Label>Manual Commentators</Form.Label>
        <Select styles={customStyles}  value={Array.from(eventState.commentators, (personID) => ({value: personID, label: people.get(personID)!.name}))} 
        onChange={selectedOptions => {
          let commentators = []
  
          for(const entry of selectedOptions){
              commentators.push(entry.value);
          }
  
          setEventState({
            ...eventState,
            commentators: commentators
          });
        }}
        options={commentatorOptions} id="eventeditcommentators" isMulti={true}></Select>
      </Row>
      <div className='d-flex'>
        <div className="p-2">
          <Button variant="primary" type="submit">Save Changes</Button>
        </div>
        <div className="p-2">
          <Button variant="danger" onClick={() => {
            var result = confirm("Are you sure you want to delete this event?");
            if (result) {
              doPost('event','DELETE',{
                id: eventState.id
              });
              setWebUIState({...webuistate, selectedEvent: -1});
            }}}>Delete Event</Button>
        </div>
        <div className="p-2">
          <Button variant="success" onClick={() => {
                setEventState({...eventState, complete: !eventState.complete});
            }}>{eventState.complete ? "Mark Event as Not Complete" : "Mark Event as Complete"}</Button>
        </div>
      </div>
      
    </Form>
    
    </>);
  }
