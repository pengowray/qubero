/*
 * PropertyPage.java
 *
 * Created on July 9, 2003, 11:23 PM
 */

package net.pengo.propertyEditor;


import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;

import java.util.*;
import java.awt.*;
import javax.swing.*;
import javax.swing.text.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;
/**
 *
 * @author  Smiley
 */
public abstract class PropertyPage extends JPanel {
    //fixme: maybe have a standard template layout someday?
    //fixme: make into a resource listener!
    
    /** Creates a new instance of PropertyPage */
    public PropertyPage() {
        super();
    }
    
    // save changes (apply/okay)
    abstract public void save();
    
    // refresh the page after save changes
    public void build() {
        return;
    }
    
    // call when the page is modified, so it knows it should be saved.
    // can also be called if the page is selected by MethodSelectionPage
    public void mod() {
	return;
    }

	// checks that properties are OK and ready to be sxtracted.
    public boolean isValid() {
        return true; // success
    }
    
    abstract public String toString();
    
    
}
