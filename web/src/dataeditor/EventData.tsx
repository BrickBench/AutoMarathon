import { useContext, useEffect, useState } from "react";
import { doPost } from "../Api";
import { Event, Person } from "../websocket";
import { Button, Col, Container, FormControl, FormLabel, InputGroup, Row } from "react-bootstrap";
import InputGroupText from "react-bootstrap/esm/InputGroupText";
import { FastDataContext } from "../Context";

function EventDataFieldInput({typename,customTemp,key2,setCustomTemp,horizontal,runner,event,setEventTemp}: {typename:any,customTemp: any,key2:any,setCustomTemp:any,horizontal:boolean,event:any,runner:any,setEventTemp:any}){
  const [inputState,setInputState]  = useState(customTemp[key2]);
  useEffect(() => {
      setInputState(customTemp[key2]);
  }, [customTemp]);
  return horizontal ? 
  <InputGroup>
    <InputGroupText>{key2}</InputGroupText>
    <FormControl type="text" name={key2} onChange={({ target }) => {
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
  </InputGroup> : 
  <Col lg={2}>
    <FormLabel>{key2}</FormLabel>
    <FormControl type="text" name={key2} onChange={({ target }) => {
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
</Col>;
}

function EventRunnerData({runnerID, event, setEventState, data, people}:{runnerID: number, event:Event, setEventState:any,data:any, people: Map<number, Person>}){
    const [eventRunnerState,setEventRunnerStateTemp] = useState(data ? data : {
                "SplitTimes": {final_result: "", splits: Array.from({ length: 36 }, (element, index) => {
                    return null;
        })}});
    useEffect(() => {
        setEventRunnerStateTemp(data ? data : 
            {
                "SplitTimes": {final_result: "", splits: Array.from({ length: 36 }, (element, index) => {
                    return null;
            })}
        });
    }, [data])

    const resultType = "SplitTimes";

    return (
    <Row className="pt-2">
        <Col lg={2}>
            {people.get(runnerID)!.name}
        </Col>
        <Col lg={10}>
            {eventRunnerState[resultType] &&
            <Row>
                {Object.entries(eventRunnerState[resultType])
                .filter(([key,val])=>{
                    return key == "final_result";
                }).map(([key,val])=>{return <EventDataFieldInput key={key} typename={resultType} runner={runnerID} horizontal={true} customTemp={eventRunnerState[resultType]} event={event} setEventTemp={setEventState} key2={key} setCustomTemp={setEventRunnerStateTemp}></EventDataFieldInput>})}
            </Row>
            }
        </Col>
    </Row>
    );
}

export function EditEventData({event, people}: {event: Event, people: Map<number, Person>}){
  const { fastData, setFastDataState } = useContext(FastDataContext);
  const [eventState, setEventState] = useState(structuredClone(event));
  useEffect(() => {
    setEventState(structuredClone(event))
  }, [event]);
  return <Container>
        <Row className="pt-2">
            <Col>
                <h2>Edit Event Data</h2>
            </Col>
            <Col className="align-items-center me-auto">
                <Button variant="primary" onClick={() => {
                    let eventCopy = structuredClone(eventState);
                    
                    let realTimeEvent = fastData.find((e)=>{e.id == event.id});
                    if(realTimeEvent){
                        let editedRunnerState = structuredClone(eventCopy.runner_state);
                        eventCopy.runner_state = realTimeEvent.runner_state;
                        for(let entry in Object.entries(eventCopy.runner_state)){
                            if(eventCopy.runner_state[entry[0]]){
                                eventCopy.runner_state[entry[0]].result["SplitTimes"]['final_result'] = entry[1].result["SplitTimes"]['final_result'];
                            } 
                        }
                    }
                    
                    doPost('event','PUT', eventCopy);
                }}>Save Changes</Button>
            </Col>
        </Row>
        {Object.entries(eventState.runner_state).map(([key,val])=>
            <EventRunnerData key={key} runnerID={parseInt(key)} event={eventState} setEventState={setEventState} data={val.result} people={people}></EventRunnerData>)
        }
    </Container>;
}