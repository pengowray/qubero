/*
 * SelectionResource.java
 *
 * Created on 4 September 2002, 19:40
 */

/**
 *
 * @author  Peter Halasz
 */
package net.pengo.resource;
import net.pengo.app.*;
import net.pengo.selection.*;

import javax.swing.*;

public class SelectionResource extends Resource implements LongListSelectionListener {
    //final protected Data sel;
    
    public SelectionResource(OpenFile openFile) {
	super(openFile);
    }
    
    public JMenu getJMenu() {
        JMenu m = new JMenu(this.getClass().getName());
        m.add(this.getClass().getName());
        return m;
    }
    
    
    /*
    public Data getData() {
        return sel;
    }
    
    public TransparentData getTransparentData() {
        if (sel instanceof TransparentData) {
            return (TransparentData)sel;
        }
        
        return sel.getTransparentData();
    }
    */
    
    //FIXME:; reimplement?
    /*
    public boolean equals(SelectionResource o) {
        if (o == this)
            return true;
        
        openFile.getSelectionModel().eq
        return (sel.getStart() == o.sel.getStart() && sel.getLength() == o.sel.getLength());
    }
     
    public boolean equals(Object o) {
        if (o == this)
            return true;
        
        if (o instanceof SelectionResource) {
            return equals((SelectionResource)o);
        }
        
        return false;
    }
    */
    
    /**
     * Called whenever the value of the selection changes.
     * @param e the event that characterizes the change.
     *
     */
    public void valueChanged(LongListSelectionEvent e) {
    }
    
}
