import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;

abstract class DefinitionResource extends Resource {

    public DefinitionResource(OpenFile openFile) {
	super(openFile);
     }

    abstract public JMenu getJMenu();

}
