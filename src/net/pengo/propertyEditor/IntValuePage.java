/*
 * ValuePage.java
 *
 * Used when editing IntegerResources (including IntPrimativeResource and
 * IntAddressedResource).
 *
 * Created on July 11, 2003, 11:51 PM
 */

package net.pengo.propertyEditor;

import java.io.IOException;

import javax.swing.JLabel;
import javax.swing.JTextField;

import net.pengo.resource.IntResource;
/**
 *
 * @author  Peter Halasz
 */
public class IntValuePage extends EditablePage {
    private IntResource res;
    private JTextField inputField;
    
    /** Creates a new instance of ValuePage */
    public IntValuePage(IntResource res) {
        this(res, null);
    }
    
    public IntValuePage(IntResource res, PropertiesForm form) {
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
//		System.out.println("saveOp() IntValuePage for: " + res + "...");
        try {
			System.out.println(" - setting value to: " + inputField.getText());
            res.setValue(inputField.getText());
        } catch (IOException e ) {
            //fixme
            e.printStackTrace();
        }
//		System.out.println(" - saveOp() complete.");
    }
    
    public void buildOp() {
		//System.out.println("building IntValuePage for: " + res + "..." );
        inputField.setText( res.getValue() + "" );
		//System.out.println(" - build complete.");
    }
    
    public boolean isValid() {
        //fixme
        return true; // success
    }
    
    public String toString() {
        return "Value";
    }
}
