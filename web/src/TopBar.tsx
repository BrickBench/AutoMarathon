import { useContext } from "react";
import { WebUIStateContext } from "./Context";
import { Event, EventTab, LockState, StreamHost } from "./websocket";
import { Nav, Navbar, NavLink, NavItem} from "react-bootstrap";
import Select from 'react-select';
import { customStyles } from "./Globals";

export function PanelSelector({selectedEvent, releaseLock} : {selectedEvent: Event | undefined, releaseLock : ()=>void}){
    const { webuistate, setWebUIState } = useContext(WebUIStateContext);
  
    return (
      <div className="d-flex pt-2 ps-2">
        <div className="pe-5">
            <h5 className="align-middle">{selectedEvent!.name}</h5>
        </div>
        <Nav variant="pills">
            <NavItem></NavItem>
            <NavItem>
                <NavLink active={webuistate.eventSubmenu == EventTab.Setup} onClick={() => {
                    setWebUIState({
                        ...webuistate,
                        eventSubmenu: EventTab.Setup
                    });
                    releaseLock();
                }}>Event Setup</NavLink>
            </NavItem>
            <NavItem>
                <NavLink active={webuistate.eventSubmenu == EventTab.Data} onClick={() => {
                    setWebUIState({
                        ...webuistate,
                        eventSubmenu: EventTab.Data
                    });
                    releaseLock();
                }}>Results</NavLink>
            </NavItem>
            <NavItem>
                <NavLink active={webuistate.eventSubmenu == EventTab.Dashboard} onClick={() => {
                    setWebUIState({
                        ...webuistate,
                        eventSubmenu: EventTab.Dashboard
                    });
                    releaseLock();
                }}>Dashboard</NavLink>
            </NavItem>
        </Nav>
      </div>
     
    );
  }
  
export function HostSelector({hosts, releaseLock} : {hosts : Map<string, StreamHost>, releaseLock : ()=>void}){
    const { webuistate, setWebUIState } = useContext(WebUIStateContext);

    let hostOptions = [...hosts.entries()].filter(([_, host])=>{return host.connected}).map(([key]) => ({value: key, label: key}));
  
    return (
      <Navbar>
        <Nav variant="pills" className="ms-auto">
            <NavItem>
                <NavLink active={webuistate.eventSubmenu == EventTab.Stream} disabled={webuistate.selectedHost == ""} onClick={() => {
                    if(webuistate.selectedHost != ""){
                        setWebUIState({
                        ...webuistate,
                        eventSubmenu: EventTab.Stream
                        });
                    }
                    }}>Host Control</NavLink>
            </NavItem>
            <NavItem className="ps-4">
                <Select styles={customStyles} value={{value:webuistate.selectedHost, label:webuistate.selectedHost}} options={hostOptions} 
                        onChange={selectedOption => {
                            setWebUIState({
                                ...webuistate,
                                selectedHost: selectedOption ? selectedOption.value : ""
                            });
                    }} ></Select>
            </NavItem>
        </Nav>
      </Navbar>
    );
 }