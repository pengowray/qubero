/*
 * IntResourceForm.java
 *
 * Created on July 9, 2003, 10:21 PM
 *
 * another attempt at IntInputBox
 */

package net.pengo.propertyEditor;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;
import net.pengo.resource.*;

import java.util.*;
import java.awt.*;
import java.awt.event.*;
import javax.swing.*;
import javax.swing.text.*;
import javax.swing.tree.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;
/**
 *
 * @author  Smiley
 */
public class IntResourcePropertiesForm extends AbstractResourcePropertiesForm {
    //private OpenFile openFile;
    private IntResource intRes;
   
    //String name;
    //private Data value;
    //private int size; // make dereferencable f(x)
    //int signed; ; // make dereferencable f(x)
    
    //private Editor;// = value+size, value, or this;
    
    /** Creates a new instance of IntResourceForm */
    //public IntResourceForm(OpenFile openFile) {
//        this.openFile = openFile;
//    }
    
    
    public IntResourcePropertiesForm(IntResource intRes) {
        super();
        this.intRes = intRes;
    }
    
    // override this.
    protected PropertyPage[] getMenu() {
        return new PropertyPage[] {
            new SummaryPage(intRes), 
            new SetTypePage(intRes, this), 
            new SizePage(intRes, this),
            new ValuePage(intRes, this),
            new AddressPage(intRes, this)
        };
    }
    
}
