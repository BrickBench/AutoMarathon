import { Button, ButtonGroup, ButtonToolbar, Card, CardBody, CardHeader, CardTitle, Col, Container, FormControl, FormSelect, InputGroup, Row, Table } from "react-bootstrap";
import { ArrowClockwise } from "react-bootstrap-icons";
import { Event, Person, Runner, RunnerRunState, RunSocketState } from "../websocket";
import Select from "react-select";
import { customStyles } from "../Globals";
import { useContext, useEffect, useState } from "react";
import { doPost } from "../Api";
import { FastDataContext } from "../Context";

function timeToMillis(editTimerString){

    if(!editTimerString)
    {
        return null;
    }

    let regex =  /^(\d+\:)?\d+\:\d+\.\d+$/;
    let time = editTimerString;
    if(!time.match(regex)){
        alert("Invalid time format (" + editTimerString + "). Please input the new time in the format HH:MM:SS.xxx.")
        return null;
    }else{
        let tokens = time.split(":");
        var timeSepCount = tokens.length - 1;
        let hours = timeSepCount >= 2 ? parseInt(tokens[0]) : 0;
        let minutes = timeSepCount >= 2 ? parseInt(tokens[1]) : parseInt(tokens[0]);
        let secmillis = (timeSepCount >= 2 ? tokens[2] : tokens[1]).split(".");
        let seconds = parseInt(secmillis[0]);
        let millis = parseInt(secmillis[1]);

        let submillissplitindex = Math.min(3, secmillis[1].length);
        let millisPart = secmillis[1].substring(0, submillissplitindex).padEnd(3,"0");
        let subMillisPart = "0." + secmillis[1].substring(submillissplitindex);

        let setstarttime;
        let setendtime;

        let millisdiff = ((hours * 60 + minutes) * 60 + seconds) * 1000 + parseInt(millisPart) + (parseFloat(subMillisPart));

        return millisdiff;
    }
}

function millisToTime(time, defaultNone=""){

  if(!time){
    return defaultNone;
  }

  let millisElapsed = time;

  let hours = Math.floor((millisElapsed / (1000 * 60 * 60)) % 24);
  let minutes = Math.floor((millisElapsed / 1000 / 60) % 60);
  let seconds = Math.floor((millisElapsed / 1000) % 60);
  let millis = millisElapsed % 1000;

  let hoursS = (hours < 10) ? "0" + hours : hours;
  let minutesS = (minutes < 10) ? "0" + minutes : minutes;
  let secondsS = (seconds < 10) ? "0" + seconds : seconds;
  let subMillis = (millis % 1).toString();
  let millisS = millis.toString().padStart(3, "0").replace(".","");
  millisS = millisS.substring(0, Math.min(6, millisS.length));
  
  return hoursS + ":" + minutesS + ":" + secondsS + "." + millisS;
}

function fillOverrideFromLive(selectedPlayerLiveRunData : RunnerRunState | undefined, editorSplits : string[]){
    
    if(!selectedPlayerLiveRunData){
        return editorSplits;
    }

    for(var i=0;i < selectedPlayerLiveRunData.splits.length; i++){
        if(i < editorSplits.length){
            editorSplits[i] = millisToTime(selectedPlayerLiveRunData.splits[i].splitTime);
        }
    }
    return editorSplits;
}

function CustomSplitsFieldInput({splitsState, splitIndex, setSplitsState}: {splitsState : string[], splitIndex : number, setSplitsState: any}){
    return <>
      <input type="text" className="form-control" onChange={({ target }) => {
          var newSplitsState = [...splitsState];
          newSplitsState[splitIndex] = target.value;
          setSplitsState(newSplitsState);
        }} value={splitsState[splitIndex] || ''}/>
    </>
    ;
}

export function SplitsEditorWidget({liveRunState, runners, people, currentEvent} : 
    {liveRunState: RunSocketState,  runners: Map<number, Runner>, people: Map<number, Person>, currentEvent : Event | undefined}){
    const { fastData, setFastDataState } = useContext(FastDataContext);

    if(!currentEvent){
         return <Card>
            <CardHeader>Splits Editor</CardHeader>
            <CardBody>No current event selected.</CardBody>
        </Card>
    }

    let eventRunners = Object.entries(currentEvent.runner_state);
    if(eventRunners.length == 0){
        return <Card>
            <CardHeader>Splits Editor</CardHeader>
            <CardBody>Event has no runners.</CardBody>
        </Card>
    } 
 
    let runnerSelectOptions = currentEvent ? Array.from(Object.entries(currentEvent.runner_state)
        , ([key, value]) => ({ value: parseInt(key), label: people.get(runners.get(parseInt(key))?.participant)!.name })) : [];
    let liveDataSourceOptions = [{label: "Use Live Data", value: 1}, {label: "Use Overrides", values: 0}];

    const [selectedPlayerState, setSelectedPlayerState] = useState<{ value: number, label: string } | undefined>(runnerSelectOptions[0]);
    let runner = selectedPlayerState ? runners.get(selectedPlayerState.value) : undefined;

    const [selectedDataState, setSelectedDataState] = useState<{value: number, label: string}>(runner ? (liveDataSourceOptions[runner.use_live_data ? 0 : 1])
        : liveDataSourceOptions[0]);

    let streamEvent = fastData ? fastData.find(element => element.id == currentEvent.id) : undefined;
    let runnerEventData = streamEvent ? streamEvent.runner_state[selectedPlayerState.value].result : undefined;

    const [manualOverrideState, setManualOverrideState] = useState<RunnerRunState>(new RunnerRunState());
    const [splitsState, setSplitsState] = useState<string[]>(runnerEventData ? runnerEventData.SplitTimes.splits.map((e)=>{
        return millisToTime(e);
    }) : []);

    useEffect(() => {
        if(selectedDataState.value == 1){
            let streamEvent = fastData ? fastData.find(element => element.id == currentEvent.id) : undefined;
            let runnerEventData = streamEvent ? streamEvent.runner_state[selectedPlayerState.value].result : undefined;
            if(runnerEventData && runnerEventData.SplitTimes){
                setSplitsState(runnerEventData.SplitTimes.splits.map((e)=>{
                    return millisToTime(e);
                }));
            }
        }
      }, [selectedPlayerState, selectedDataState, fastData])

    let selectedPlayerLiveRunData = selectedPlayerState && liveRunState && liveRunState.active_runs ? liveRunState.active_runs[selectedPlayerState.value] : undefined;

    return <Card>
        <CardHeader>Splits Editor</CardHeader>
        <CardBody>
        <Container>
          <Row>
            <Col>
                <Select value={selectedPlayerState} styles={customStyles}
                            options={runnerSelectOptions} onChange={(option) => {setSelectedPlayerState(option);}}></Select>
            </Col>
            <Col>
                <Select value={selectedDataState} styles={customStyles} options={liveDataSourceOptions} onChange={
                    (option) => {
                        setSelectedDataState(option);
                    }
                }></Select>
            </Col>
            <Col>
                <Button onClick={()=>{
                    if(streamEvent && selectedPlayerState){
                        var runnerState = {...streamEvent?.runner_state};
                        var playerResults = runnerState[selectedPlayerState.value];

                        var finalResults;

                        if(playerResults.result){
                            finalResults = {...playerResults.result};
                            finalResults.SplitTimes.splits = splitsState.map((e)=>{
                                return timeToMillis(e);
                            });
                        }else{
                            finalResults = {"SplitTimes": {final_result: "", splits: Array.from({ length: 36 }, (element, index) => {
                                        return null;
                                })}};
                        }
                        
                        runnerState[selectedPlayerState.value].result = finalResults;
                        
                        doPost('event', 'PUT', {...currentEvent, runner_state: runnerState})
                    }
                    
                    doPost('runner', 'PUT', { ...runner, use_live_data: (selectedDataState.value == 1) });
                }}>Push Changes</Button>
            </Col>
            </Row>
            <Row className="pt-2">
                <ButtonGroup >
                    <Button variant="secondary" onClick={()=>{
                        if(selectedPlayerState)
                            doPost('runner/splits/refresh', "POST", {id: selectedPlayerState.value});
                    }}
                    ><ArrowClockwise/> Live Data</Button>
                    <Button onClick={()=>{
                        setSplitsState(fillOverrideFromLive(selectedPlayerLiveRunData, [...splitsState]));
                    }}>Fill Overrides from Live</Button>
                    <Button variant={"danger"} onClick={()=>{
                        var splits = runnerEventData ? runnerEventData.SplitTimes.splits.map((e)=>{
                            return null;
                        }) : [];
                        let confirm = prompt("Are you sure you want to clear? If so, type \"Clear\" into the box and press \"Push Changes\" on the widget.");
                        if(confirm?.toLowerCase().replace("\"","").replace("'","") == "clear"){
                            setSplitsState(splits);
                        }
                    }}>Clear Data</Button>
                </ButtonGroup>
            </Row>
            <div style={{ height: '300px', overflowY: 'auto' }}>
          <Table>
            <thead style={{ position: "sticky", top: 0}}>
                <tr>
                    <th>Split</th>
                    <th>Live Data</th>
                    <th>Manual Override</th>
                </tr>
            </thead>
            <tbody style={{zIndex: 1}}>
                {manualOverrideState ? 
                    manualOverrideState.splits.map((split, index)=>{
                        return <tr key={index}>
                            <td>{(Math.floor(index/6)+1) + "-" + (Math.floor(index%6)+1)}</td>
                            <td>{
                                selectedPlayerLiveRunData ?
                                    ((index < selectedPlayerLiveRunData.splits.length) ? 
                                        millisToTime(selectedPlayerLiveRunData.splits[index].splitTime) :
                                    <>--:--</>)
                                : <>--:--</> 
                            }</td>
                            <td>
                                <CustomSplitsFieldInput splitsState={splitsState} setSplitsState={setSplitsState}
                                    splitIndex={index}/>
                            </td>
                        </tr>
                    })
                 : <></>}
            </tbody>
          </Table>
          </div>
        </Container>
        </CardBody>
        </Card>;
}