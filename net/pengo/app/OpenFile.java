package net.pengo.app;
import net.pengo.resource.*;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedList;
import java.util.List;
import javax.swing.event.EventListenerList;
import net.pengo.data.Data;
import net.pengo.data.DataListener;
import net.pengo.data.EditableData;
import net.pengo.restree.ResourceList;
import net.pengo.selection.LongListSelectionEvent;
import net.pengo.selection.LongListSelectionListener;
import net.pengo.selection.LongListSelectionModel;
import net.pengo.selection.SegmentalLongListSelectionModel;
import net.pengo.selection.SimpleLongListSelectionModel;

/**
 * Node used by MoojTree containing display information for a file/definition.
 * also to be uesd by HexPanel?
 *
 * see also CurrentOpenFile
 */

public class OpenFile implements LongListSelectionListener { // previously did extend DefaultMutableTreeNode
    
    
    /**
     * Called whenever the value of the selection changes.
     * @param e the event that characterizes the change.
     */
    public void valueChanged(LongListSelectionEvent e) {
	// TODO
    }
    
    protected Data rawdata;
    protected String filename;
    protected LongListSelectionModel selectionModel;
    
    protected EventListenerList listenerList = new EventListenerList();

    private final ResourceFactory resFact = new OpenFileResourceFactory(this);
    private ResourceList rootResList;
    private List definitionResList = new ResourceList(Collections.synchronizedList(new LinkedList()), resFact , "Definitions") ;
    private List selectionDetails = new ResourceList(Collections.synchronizedList(new LinkedList()), resFact , "Selection details");
    private LiveSelectionResource liveSelection;
    
    private ActiveFile af;
    /**
     * def may be null. in future rawdata may be null too (to indicate an empty file).
     */
    
    public OpenFile(Data rawdata) {
        this(rawdata, null);
    }
    
    public OpenFile(Data rawdata, ActiveFile af) {
	this.rawdata = rawdata;
        this.af = af;
        rootResList = new ResourceList(new ArrayList(), resFact , this+"");
	rootResList.add(definitionResList);
	rootResList.add(selectionDetails);
	addLongListSelectionListener(this);
	liveSelection = new LiveSelectionResource(this)
        getResourceList().add(liveSelection); // adds selection. //xxx: should this be here?
    }
    
    public ActiveFile getActiveFile() {
        return af;
    }
    
    public void setActiveFile(ActiveFile af) {
        this.af = af;;
    }
    
    public void makeActive(Object source) {
        af.setActive(this, source);
    }

    public ResourceFactory getResourceFactory() {
	return resFact;
    }
    
    public void addDataListener(DataListener l) {
	if (rawdata instanceof EditableData) {
	    ((EditableData)rawdata).addDataListener(l);
	}
	//fixme: else ignore, wont be any changes?
    }
    
    public void removeDataListener(DataListener l) {
	if (rawdata instanceof EditableData) {
	    ((EditableData)rawdata).removeDataListener(l);
	}
	//fixme: else ignore, wont be any changes?
    }
    
	
    // register with this instead of with the actual LongListSelection. Tho both should work I guess.
    public void addLongListSelectionListener(LongListSelectionListener l) {
	listenerList.add(LongListSelectionListener.class, l);
    }
    
    //
    public void removeLongListSelectionListener(LongListSelectionListener l) {
	listenerList.remove(LongListSelectionListener.class, l);
    }
    
    public List getDefinitionList() {
	return definitionResList;
    }
    
    public List getSelectionDetails() {
	return selectionDetails;
    }
    
    public ResourceList getResourceList() {
	return rootResList;
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
	if (resource == null)
	    return;
	
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
    
    //fixme: perhaps "SegmentalLongListSelectionModel" should replaced with "EditableLongListSelectionModel"
    public void setSelectionModel(LongListSelectionModel selectionModel) {
	//System.out.println("Setting selection model..." + selectionModel);
	
	if (this.selectionModel != null) // && this.selectionModel != selectionModel
	{
	    this.selectionModel.removeLongListSelectionListener(this);
	    //getResourceList().remove(this.selectionModel);
	    //fireResourceRemoved(this,"Selection",this.selectionResource);
        }
        
	if (selectionModel instanceof SimpleLongListSelectionModel) {
	    selectionModel = ((SimpleLongListSelectionModel)selectionModel).toSegmental();
	}
	
	this.selectionModel = selectionModel;
	selectionModel.setEventListenerList(listenerList);
	if (selectionModel == null ) { // || selectionModel.isSelectionEmpty()
	    return;
	}
	
	
        
	//selectionResource = new LiveSelectionResource(this); //fixme: probably unnecessary
	//fireResourceAdded(this,"Selection",selectionResource);
	
        // no longer needed.
        //getResourceList().add(this.selectionModel);
        
	//fixme: better way to fire?
	//selectionModel.setValueIsAdjusting(false);
	//selectionModel.setValueIsAdjusting(true);
	
	liveSelection.updated();
	
    }
    
    public LongListSelectionModel getSelectionModel() {
	if (selectionModel == null) {
	    setSelectionModel(new SegmentalLongListSelectionModel());
	}
	return selectionModel;
    }
    
    public boolean close(Object source) {
	//fixme fixme fixme
	fireFileClosed(source);
	// should we do anything else??
	//FIXME: confirm changes?
	//FIXME: throw exceptions??
	return true;
    }
    
    public void saveAs(Object source, File filename) throws IOException {
	//FIXME: use a pipe?
	InputStream in = rawdata.dataStream();
	FileOutputStream out = new FileOutputStream(filename, false);
	
	int c;
	while ((c = in.read()) != -1) {
	    out.write(c);
	}
	
	out.close();
	
	fireFileSaved(source);
    }
    
    public Data getData() {
	return rawdata;
    }
    
    public void setData(Data data) {
	//FIXME: should trigger a few things
	this.rawdata = rawdata;
    }
    
    public EditableData getEditableData() {
	//FIXME: this is ugly as shit
	if (rawdata instanceof EditableData) {
	    return (EditableData)rawdata;
	}
	else {
	    
	    //FIXME: throw a wobbly.
	    return null;
	}
    }
    
    public String toString() {
	return rawdata.toString();
    }
    
}
