package net.pengo.app;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedList;
import java.util.List;

import javax.swing.event.EventListenerList;
import net.pengo.bitSelection.BitSelectionModel;
import net.pengo.bitSelection.SegmentalBitSelectionModel;

import net.pengo.data.Data;
import net.pengo.data.DataListener;
import net.pengo.data.EditableData;
import net.pengo.resource.LiveSelectionResource;
import net.pengo.resource.OpenFileResourceFactory;
import net.pengo.resource.Resource;
import net.pengo.resource.ResourceEvent;
import net.pengo.resource.ResourceFactory;
import net.pengo.resource.ResourceListener;
import net.pengo.restree.ResourceList;
import net.pengo.selection.LongListSelectionEvent;
import net.pengo.selection.LongListSelectionListener;
import net.pengo.selection.LongListSelectionModel;
import net.pengo.selection.MetaSelectionModel;
import net.pengo.selection.SegmentalLongListSelectionModel;

/**
 * Node used by MoojTree containing display information for a file/definition.
 * also to be used by HexPanel?
 *
 * see also ActiveFile
 */

public class OpenFile extends Resource implements LongListSelectionListener { // previously did extend DefaultMutableTreeNode
    protected Data rawdata;
    protected String filename;
    protected MetaSelectionModel selectionModel;
    
    protected SegmentalBitSelectionModel selection;
    
    protected EventListenerList listenerList = new EventListenerList();
    
    private final ResourceFactory resFact = new OpenFileResourceFactory(this);
    private ResourceList rootResList;
    private List definitionResList = new ResourceList(Collections.synchronizedList(new LinkedList()), resFact , "Definitions") ;
    private List selectionDetails = new ResourceList(Collections.synchronizedList(new LinkedList()), resFact , "Selection details");

	//fixme: eventually replace liveSelection (LiveSelectionResource) with SmartPointer.. and replace metaselectionmodel thing
    //final public SmartPointer selection = new JavaPointer("net.pengo.resource.LiveSelectionResource");
 
    public final LiveSelectionResource liveSelection = new LiveSelectionResource(this);
    
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
		//selection.setValue(live);

        getResourceList().add(liveSelection); //fixme: should this be here?
    }
    
    public ActiveFile getActiveFile() {
        return af;
    }
    
    public void setActiveFile(ActiveFile af) {
        this.af = af;
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
        //FIXME: else ignore, wont be any changes?
    }
    
    //* use this method rather than doing it on the selection directly */
    public void addLongListSelectionListener(LongListSelectionListener l) {
        getSelectionModel().addLongListSelectionListener(l);
    }
    
    public void removeLongListSelectionListener(LongListSelectionListener l) {
        getSelectionModel().removeLongListSelectionListener(l);
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
                    event = new ResourceEvent(source,this,category,resource);
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
                    event = new ResourceEvent(source,this,category,resource);
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
    
    public void setSelectionModel(LongListSelectionModel sm) {
//
        if (selectionModel == null) {
            selectionModel = new MetaSelectionModel(sm);
            selectionModel.addLongListSelectionListener(this);
        } else {
            selectionModel.setModel(sm);
        }
        
        //liveSelection.updated();
    }
    
    /** old */
    public LongListSelectionModel getSelectionModel() {
        if (selectionModel == null) {
            setSelectionModel(new SegmentalLongListSelectionModel());
        }
        return selectionModel;
    }

    public SegmentalBitSelectionModel getSelection() {
        if (selection == null) 
            setSelection(new SegmentalBitSelectionModel());
        
        return selection;
    }

    public void setSelection(SegmentalBitSelectionModel sm) {
        selection = sm;
        
        //liveSelection.updated();
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
        this.rawdata = data;
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
    
    /**
     * Called whenever the value of the selection changes.
     * @param e the event that characterizes the change.
     */
    public void valueChanged(LongListSelectionEvent e) {
        // TODO
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.Resource#getSources()
     */
    public Resource[] getSources() {
        // TODO Auto-generated method stub
        return null;
    }

    public void editProperties() {
        // TODO Auto-generated method stub
        
    }

    public void doubleClickAction() {
        super.doubleClickAction();
        makeActive(this);
    }
    
}
