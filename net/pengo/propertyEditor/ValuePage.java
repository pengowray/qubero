/*
 * ValuePage.java
 *
 * Created on July 11, 2003, 11:51 PM
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
import java.awt.event.*;
/**
 *
 * @author  Smiley
 */
public class ValuePage extends EditablePage {
    private IntResource res;
    private JTextField inputField;
    
    /** Creates a new instance of ValuePage */
    public ValuePage(IntResource res, IntResourcePropertiesForm form) {
        super(form);
        this.res = res;
        this.form = form;

        add(new JLabel( "Value: " ));
        inputField = new JTextField("0",12);
        inputField.addActionListener( getSaveActionListener() );
        inputField.getDocument().addDocumentListener( this );
            
        add(inputField);
        build();
    }
    
    
    public void saveOp() {
        res.setValue(inputField.getText());
    }
    
    public void buildOp() {
        inputField.setText( res.getValue() + "" );
    }
    
    public boolean isValid() {
        //fixme
        return true; // success
    }    
    
    public String toString() {
        return "Value";
    }    
}
