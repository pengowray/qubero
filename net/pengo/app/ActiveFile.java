/**
 * ActiveFile.java
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.app;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.Iterator;
import java.util.Set;
import net.pengo.app.*;
import net.pengo.data.Data;
import net.pengo.resource.ResourceFactory;
import net.pengo.restree.ResourceList;

// currently viewed file by a component or set of components.
// e.g. A shared panel in an MDI still only has one ActiveFile, but.. you get the idea

// OpenFile's cannot be shared between ActiveFile available lists

public class ActiveFile {
    OpenFile active; // currently active file
    ResourceList available = new ResourceList(new ArrayList(), (ResourceFactory)null, "Active files"); // all OpenFile's available to edit
    Set listeners = new HashSet(); //
    
    public ActiveFile() {
	super();
    }
	
    public ActiveFile(OpenFile d) {
	this();
	setActive(d, ActiveFile.class);
    }
    public ActiveFile(Data d) {
	this();
	setActive(new OpenFile(d, this), null);
    }
    
    public ResourceList getResourceList() {
	return available;
    }
    
    public void addActiveFileListener(ActiveFileListener l) {
	listeners.add(l);
    }
    
    public void removeActiveFileListener(ActiveFileListener l) {
	listeners.remove(l);
    }
    public void closeAll(Object source) {
	for (Iterator i = available.iterator(); i.hasNext(); ) {
	    OpenFile of = (OpenFile)i.next();
	    close(of, source);
	}
    }
    
    // closes the file and remove from list
    public boolean close(OpenFile closeFile, Object source) {
	//fixme: not complete
        if (closeFile == active) {
	    setActive(null, source);
	}
	
	if (!available.contains(closeFile)) {
	    //fixme: not really illegal argument ?
	    throw new IllegalArgumentException("file to close is not even open");
	}

	boolean ready = true;
	ActiveFileEvent e = new ActiveFileEvent(source, this, active);
	for (Iterator i=listeners.iterator(); i.hasNext() && ready==true; ) {
	    ActiveFileListener l = (ActiveFileListener)i.next();
	    if (!l.readyToCloseOpenFile(e)) {
		ready = false;
	    }
	}
	
        //close it now!
        if (!ready) {
            System.out.println("not ready to close");
            return false;
        }
            
        closeFile.setActiveFile(null);
	boolean done = active.close(source);
	
        if (!done) {
            System.out.println("failed to close");
            return false;
        }
        
        for (Iterator i=listeners.iterator(); i.hasNext(); ) {
            ActiveFileListener l = (ActiveFileListener)i.next();
            l.closedOpenFile(e);
        }
        
	return true;
    }
    
    // sets "active" as the active OpenFile. If not in the available list, it is added.
    public void setActive(OpenFile active, Object source) {
        if (active != null && !available.contains(active)) {
	    available.add(active);
            active.setActiveFile(this);
	}
	
	this.active = active;
	
	ActiveFileEvent e = new ActiveFileEvent(source, this, active);
	for (Iterator i=listeners.iterator(); i.hasNext(); ) {
	    ActiveFileListener l = (ActiveFileListener)i.next();
	    l.activeChanged(e);
	}
    }
    
    
    // may return null
    public OpenFile getActive() {
	return active;
    }
    
}
