import java.util.*;
import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;

/**
 * Node used by MoojTree containing display information for a file/definition.
 * also to be uesd by HexPanel?
 * //XXX: needs to be renamed (e.g. OpenFileNode or TopFileNode or something)
 */

class OpenFile { // previously did extend DefaultMutableTreeNode
    protected RawData rawdata;
    
    protected EventListenerList listenerList = new EventListenerList();
   
    protected HexPanel hexpanel; //XXX: allow multiple?

    protected SelectionResource selectionRes;
    protected RawDataSelection selection; //XXX: do we really need both?
    
    protected List definitionList = new LinkedList();
    protected List definitionResList = new LinkedList(); //XXX: do we really need both?
    
    /**
     * def may be null. in future rawdata may be null too (to indicate an empty file).
     */
    public OpenFile(RawData rawdata) {
	this.rawdata = rawdata;
    }


    public void addResourceListener(ResourceListener l) {
        listenerList.add(ResourceListener.class, l);
    }
 
    public void removeResourceListener(ResourceListener l) {
        listenerList.remove(ResourceListener.class, l);
    }
    
    public void addOpenFileListener(OpenFileListener l) {
        listenerList.add(OpenFileListener.class, l);
    }
 
    public void removeOpenFileListener(OpenFileListener l) {
        listenerList.remove(OpenFileListener.class, l);
    }
    
    protected void fireResourceAdded(Object source, String category, Resource resource) {
        Object[] listeners = listenerList.getListenerList();
        ResourceEvent event = null;
        for (int i = listeners.length-2; i>=0; i-=2) {
            if (listeners[i]==ResourceListener.class) {
                // Lazily create the event:
                if (event == null) {
                    if (source == null) // is this normal behaviour?
                        source = this;
                    event = new ResourceEvent(this,category,resource);
                }
                ((ResourceListener)listeners[i+1]).resourceAdded(event);
            }
        }
    }

    protected void fireResourceRemoved(Object source, String category, Resource resource) {
        Object[] listeners = listenerList.getListenerList();
        ResourceEvent event = null;
        for (int i = listeners.length-2; i>=0; i-=2) {
            if (listeners[i]==ResourceListener.class) {
                // Lazily create the event:
                if (event == null) {
                    if (source == null)
                        source = this;
                    event = new ResourceEvent(source,category,resource);
                }
                ((ResourceListener)listeners[i+1]).resourceRemoved(event);
            }
        }
    }

    protected void fireSelectionMade(Object source, RawDataSelection resource) {
        Object[] listeners = listenerList.getListenerList();
        SelectionEvent event = null;
        for (int i = listeners.length-2; i>=0; i-=2) {
            if (listeners[i]==OpenFileListener.class) {
                // Lazily create the event:
                if (event == null) {
                    if (source == null)
                        source = this;
                    event = new SelectionEvent(source,resource);
                }
                ((OpenFileListener)listeners[i+1]).selectionMade(event);
            }
        }
    }

    protected void fireDefinitionMade(Object source, RawDataSelection resource) {
        Object[] listeners = listenerList.getListenerList();
        DefinitionEvent event = null;
        for (int i = listeners.length-2; i>=0; i-=2) {
            if (listeners[i]==OpenFileListener.class) {
                // Lazily create the event:
                if (event == null) {
                    if (source == null)
                        source = this;
                    event = new DefinitionEvent(source,resource);
                }
                ((OpenFileListener)listeners[i+1]).definitionMade(event);
            }
        }
    }
    
    public void setSelection(Object source, RawDataSelection sel){
        // check that RawDataSelection is valid..
        if (sel.getOpenFile() != this)
            throw new IllegalArgumentException("Selection does not belong to this OpenFile");
        
        if (this.selection != null && this.selection.equals(sel))
            return; // no need to do anything
        
        if (this.selection != null) {
            fireResourceRemoved(source,"Selection",selectionRes);
        }
            
        this.selection = sel;
        this.selectionRes = new SelectionResource(sel);
        
        fireResourceAdded(source,"Selection",selectionRes);
        fireSelectionMade(source,selection);
    }


    // MoojTree rename (edit) of node / converting selection to a definition:
    public void addDefinition(Object source, RawDataSelection sel) {
        DefinitionResource defRes = new DefinitionResource(sel);
        definitionList.add(sel);
        definitionResList.add(defRes);
        fireResourceAdded(source,"Definition",defRes);
        fireDefinitionMade(source,sel);
    }
    
    public void deleteDefinition(Object source, RawDataSelection sel) {
        //NYI
    }

    public RawDataSelection getSelection(){
        //XXX pending
        return null;
	//return currentSelection;
    }

    public RawData getRawData() {
	return rawdata;
    }

    public void setRawData(RawData rawdata) {
	//XXX: should trigger a few things
	this.rawdata = rawdata;
    }

    public String toString() {
	return rawdata.toString() + "[file]";
	//return "DefNode " + rawdata.toString();
    }


}
