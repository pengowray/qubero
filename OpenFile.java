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
    protected Data rawdata;
    
    protected EventListenerList listenerList = new EventListenerList();
   
    protected HexPanel hexpanel; //XXX: allow multiple?

    protected SelectionResource selection;
    
    //protected List definitionList = new LinkedList(); // (do we really need both?)
    protected List definitionResList = new LinkedList(); 
    
    /**
     * def may be null. in future rawdata may be null too (to indicate an empty file).
     */
    public OpenFile(Data rawdata) {
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
    
    protected void fireFileSaved(Object source) {
        Object[] listeners = listenerList.getListenerList();
        FileEvent event = null;
        for (int i = listeners.length-2; i>=0; i-=2) {
            if (listeners[i]==OpenFileListener.class) {
                // Lazily create the event:
                if (event == null) {
                    if (source == null)
                        source = this;
                    event = new FileEvent(source,this);
                }
                ((OpenFileListener)listeners[i+1]).fileSaved(event);
            }
        }
    }
    
    protected void fireFileClosed(Object source) {
        Object[] listeners = listenerList.getListenerList();
        FileEvent event = null;
        for (int i = listeners.length-2; i>=0; i-=2) {
            if (listeners[i]==OpenFileListener.class) {
                // Lazily create the event:
                if (event == null) {
                    if (source == null)
                        source = this;
                    event = new FileEvent(source,this);
                }
                ((OpenFileListener)listeners[i+1]).fileClosed(event);
            }
        }
    }
    
    protected void fireSelectionMade(Object source, SelectionResource resource) {
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
    
    protected void fireSelectionCleared(Object source) {
        Object[] listeners = listenerList.getListenerList();
        SelectionEvent event = null;
        for (int i = listeners.length-2; i>=0; i-=2) {
            if (listeners[i]==OpenFileListener.class) {
                // Lazily create the event:
                if (event == null) {
                    if (source == null)
                        source = this;
                    event = new SelectionEvent(source,null);
                }
                ((OpenFileListener)listeners[i+1]).selectionCleared(event);
            }
        }
    }

    public void setSelection(Object source, Data data){
        setSelection(source, new DefaultSelectionResource(this, data));
    }

    public void setSelection(Object source, SelectionResource sel){
        // check that TransparentData is valid..
        if (sel.getOpenFile() != this)
            throw new IllegalArgumentException("Selection does not belong to this OpenFile");
        
        if (this.selection != null && this.selection.equals(sel))
            return; // no need to do anything
        
        if (this.selection != null) {
            fireResourceRemoved(source,"Selection",selection);
        }
            
        //this.selection = sel;
        this.selection = sel;
        
        fireResourceAdded(source,"Selection",selection);
        fireSelectionMade(source,selection);
        //fireSelectionMade(source,selection.getTransparentData());
    }

    public void clearSelection(Object source){
        if (this.selection == null)
            return; // already clear
        
        SelectionResource oldSelRes = selection;

        selection = null;
        
        fireResourceRemoved(source,"Selection",oldSelRes);
        fireSelectionCleared(source);
        
    }

    public void addBreak(Object source, DefinitionResource defRes) {
        definitionResList.add(defRes);
        fireResourceAdded(source,"Break",defRes);
    }

    public void deleteBreak(Object source, DefinitionResource defRes) {
        boolean success = definitionResList.remove(defRes);
        if (success) {
            fireResourceRemoved(source,"Break",defRes);
        } else {
            System.out.println("you dick. src:" + source + " res:" + defRes);
        }
    }
    
    // MoojTree rename (edit) of node / converting selection to a definition:
    public void addDefinition(Object source, DefinitionResource defRes) {
        definitionResList.add(defRes);
        fireResourceAdded(source,"Definition",defRes);
    }
    
    public void deleteDefinition(Object source, DefinitionResource defRes) {
        definitionResList.remove(defRes);
        fireResourceRemoved(source,"Definition",defRes);
    }
    
    public void definitionChange(Object source, DefinitionResource oldRes, DefinitionResource newRes) {
        //XXX lazy poo
        deleteDefinition(source, oldRes);
        addDefinition(source, newRes);
    }

    public void close(Object source) {
        fireFileClosed(source);
        // should we do anything else??
        //XXX: confirm changes?
        //XXX: throw exceptions??
    }

    public Data getData() {
	return rawdata;
    }

    public void setData(Data data) {
	//XXX: should trigger a few things
	this.rawdata = rawdata;
    }

    public String toString() {
	return rawdata.toString();
    }


}
