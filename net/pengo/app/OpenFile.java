package net.pengo.app;
import net.pengo.selection.*;
import net.pengo.resource.*;
import net.pengo.data.*;

import java.util.*;
import javax.swing.*;
import javax.swing.tree.*;
import javax.swing.event.*;
import java.io.*;

/**
 * Node used by MoojTree containing display information for a file/definition.
 * also to be uesd by HexPanel?
 *
 * see also CurrentOpenFile
 */

class OpenFile implements LongListSelectionListener { // previously did extend DefaultMutableTreeNode
    
    protected Data rawdata;
    
    protected EventListenerList listenerList = new EventListenerList();
	
    protected DefaultSelectionResource selectionResource; // was called 'selection'
    protected boolean isSelectiomResourcePublished = false;
    protected LongListSelectionModel selectionModel;
    
    protected String filename;
    
    //protected List definitionList = new LinkedList(); // (do we really need both?)
    protected List definitionResList = new LinkedList();
    
    /**
	 * def may be null. in future rawdata may be null too (to indicate an empty file).
	 */
    public OpenFile(Data rawdata)
	{
		this.rawdata = rawdata;
    }
	
	
    public void addResourceListener(ResourceListener l)
	{
		listenerList.add(ResourceListener.class, l);
    }
	
    public void removeResourceListener(ResourceListener l)
	{
		listenerList.remove(ResourceListener.class, l);
    }
    
    public void addOpenFileListener(OpenFileListener l)
	{
		listenerList.add(OpenFileListener.class, l);
    }
	
    public void removeOpenFileListener(OpenFileListener l)
	{
		listenerList.remove(OpenFileListener.class, l);
    }
    
    protected void fireResourceAdded(Object source, String category, Resource resource)
	{
		Object[] listeners = listenerList.getListenerList();
		ResourceEvent event = null;
		for (int i = listeners.length-2; i>=0; i-=2)
		{
			if (listeners[i]==ResourceListener.class)
			{
				// Lazily create the event:
				if (event == null)
				{
					if (source == null) // is this normal behaviour?
						source = this;
					event = new ResourceEvent(this,category,resource);
				}
				((ResourceListener)listeners[i+1]).resourceAdded(event);
			}
		}
    }
	
    protected void fireResourceRemoved(Object source, String category, Resource resource)
	{
		if (resource == null)
			return;
		
		Object[] listeners = listenerList.getListenerList();
		ResourceEvent event = null;
		for (int i = listeners.length-2; i>=0; i-=2)
		{
			if (listeners[i]==ResourceListener.class)
			{
				// Lazily create the event:
				if (event == null)
				{
					if (source == null)
						source = this;
					event = new ResourceEvent(source,category,resource);
				}
				((ResourceListener)listeners[i+1]).resourceRemoved(event);
			}
		}
    }
    
    protected void fireFileSaved(Object source)
	{
		Object[] listeners = listenerList.getListenerList();
		FileEvent event = null;
		for (int i = listeners.length-2; i>=0; i-=2)
		{
			if (listeners[i]==OpenFileListener.class)
			{
				// Lazily create the event:
				if (event == null)
				{
					if (source == null)
						source = this;
					event = new FileEvent(source,this);
				}
				((OpenFileListener)listeners[i+1]).fileSaved(event);
			}
		}
    }
    
    protected void fireFileClosed(Object source)
	{
		Object[] listeners = listenerList.getListenerList();
		FileEvent event = null;
		for (int i = listeners.length-2; i>=0; i-=2)
		{
			if (listeners[i]==OpenFileListener.class)
			{
				// Lazily create the event:
				if (event == null)
				{
					if (source == null)
						source = this;
					event = new FileEvent(source,this);
				}
				((OpenFileListener)listeners[i+1]).fileClosed(event);
			}
		}
    }
    
    public void setSelectionModel(LongListSelectionModel selectionModel)
	{
		if (this.selectionModel != null) // && this.selectionModel != selectionModel
		{
			this.selectionModel.removeLongListSelectionListener(this);
			fireResourceRemoved(this,"Selection",this.selectionResource);
		}
		
		this.selectionModel = selectionModel;
		selectionModel.addLongListSelectionListener(this);
		isSelectiomResourcePublished = false;
		
		updateSelectionResource();
    }
    
    public LongListSelectionModel getSelectionModel()
	{
		if (!isSelectionModel())
		{
			selectionModel = new SegmentalLongListSelectionModel();
		}
		return selectionModel;
    }
	
	public boolean isSelectionModel()
	{
		return (selectionModel != null);
	}
	
    /**
	 * Called whenever the value of the selection changes.
	 * @param e the event that characterizes the change.
	 *
	 */
    public void valueChanged(LongListSelectionEvent e)
	{
		// OpenFile now responsible for creating the selection resource ?
		if (!e.getValueIsAdjusting())
		{
			updateSelectionResource();
			
		}
    }
    
    
    private void updateSelectionResource()
	{
		if (selectionModel.isSelectionEmpty() && isSelectiomResourcePublished)
		{
			isSelectiomResourcePublished = false;
			fireResourceRemoved(this,"Selection",selectionResource);
		}
		else if (!selectionModel.isSelectionEmpty() && !isSelectiomResourcePublished)
		{
			if (selectionResource == null)
			{
				selectionResource = new DefaultSelectionResource(this);
			}
			isSelectiomResourcePublished = true;
			fireResourceAdded(this,"Selection",selectionResource);
		}
    }
	
    /*
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
	 */
	
    public void addBreak(Object source, DefinitionResource defRes)
	{
		definitionResList.add(defRes);
		fireResourceAdded(source,"Break",defRes);
    }
	
    public void deleteBreak(Object source, DefinitionResource defRes)
	{
		boolean success = definitionResList.remove(defRes);
		if (success)
		{
			fireResourceRemoved(source,"Break",defRes);
		}
		else
		{
			System.out.println("my bad. src:" + source + " res:" + defRes);
		}
    }
    
    // MoojTree rename (edit) of node / converting selection to a definition:
    public void addDefinition(Object source, DefinitionResource defRes)
	{
		definitionResList.add(defRes);
		fireResourceAdded(source,"Definition",defRes);
    }
    
    public void deleteDefinition(Object source, DefinitionResource defRes)
	{
		definitionResList.remove(defRes);
		fireResourceRemoved(source,"Definition",defRes);
    }
    
    public void definitionChange(Object source, DefinitionResource oldRes, DefinitionResource newRes)
	{
		//FIXME: lazy poo
		deleteDefinition(source, oldRes);
		addDefinition(source, newRes);
    }
	
    public void close(Object source)
	{
		fireFileClosed(source);
		// should we do anything else??
		//FIXME: confirm changes?
		//FIXME: throw exceptions??
    }
	
    public void saveAs(Object source, File filename) throws IOException
	{
		//FIXME: use a pipe?
		InputStream in = rawdata.dataStream();
		FileOutputStream out = new FileOutputStream(filename, false);
		
		int c;
		while ((c = in.read()) != -1)
		{
			out.write(c);
		}
		
		out.close();
		
		fireFileSaved(source);
    }
    
    public Data getData()
	{
		return rawdata;
    }
	
    public void setData(Data data)
	{
		//FIXME: should trigger a few things
		this.rawdata = rawdata;
    }
	
    public EditableData getEditableData()
	{
		//FIXME: this is ugly as shit
		if (rawdata instanceof EditableData)
		{
			return (EditableData)rawdata;
		}
		else
		{
			//FIXME: throw a wobbly.
			return null;
		}
    }
    
    public String toString()
	{
		return rawdata.toString();
    }
	
}
